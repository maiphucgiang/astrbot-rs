use crate::errors::{AstrBotError, Result};
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    pub id: String,
    pub doc_id: String,
    pub text: String,
    pub index: usize,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub content: String,
    pub metadata: Option<Value>,
}

pub struct DocumentParser;

impl DocumentParser {
    pub fn parse_text(content: &str, title: impl Into<String>) -> Document {
        Document {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
            content: content.to_string(),
            metadata: None,
        }
    }

    pub fn parse_markdown(content: &str, title: impl Into<String>) -> Document {
        let text = Self::strip_markdown(content);
        Document {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
            content: text,
            metadata: Some(serde_json::json!({"format": "markdown"})),
        }
    }

    pub fn parse_json(json_str: &str) -> Result<Document> {
        let val: Value = serde_json::from_str(json_str)
            .map_err(|e| AstrBotError::Serialization(format!("JSON parse error: {}", e)))?;
        let title = val.get("title").and_then(|v| v.as_str()).unwrap_or("untitled").to_string();
        let content = val.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok(Document {
            id: uuid::Uuid::new_v4().to_string(),
            title, content,
            metadata: Some(val),
        })
    }

    pub fn parse_pdf(bytes: &[u8], title: impl Into<String>) -> Result<Document> {
        let content = String::from_utf8_lossy(bytes);
        let re = Regex::new(r"\(([^)\\]+)\)").map_err(|e| AstrBotError::Internal(format!("Regex error: {}", e)))?;
        let mut extracted = Vec::new();
        for cap in re.captures_iter(&content) {
            if let Some(m) = cap.get(1) {
                let text = m.as_str().trim();
                if !text.is_empty() && text.len() > 1 { extracted.push(text.to_string()); }
            }
        }
        if extracted.is_empty() {
            return Err(AstrBotError::Internal("PDF text extraction failed: no text streams found. Consider adding a proper PDF parser like lopdf.".to_string()));
        }
        let content = extracted.join("
");
        Ok(Document { id: uuid::Uuid::new_v4().to_string(), title: title.into(), content, metadata: Some(serde_json::json!({"format": "pdf"})) })
    }

    fn strip_markdown(md: &str) -> String {
        let mut text = md.to_string();
        let code_block = Regex::new(r"```[\s\S]*?```").unwrap();
        text = code_block.replace_all(&text, "
").to_string();
        let inline_code = Regex::new(r"`([^`]+)`").unwrap();
        text = inline_code.replace_all(&text, "$1").to_string();
        let image = Regex::new(r"!\[([^\]]*)\]\([^\)]*\)").unwrap();
        text = image.replace_all(&text, "$1").to_string();
        let link = Regex::new(r"\[([^\]]+)\]\([^\)]*\)").unwrap();
        text = link.replace_all(&text, "$1").to_string();
        let header = Regex::new(r"^#{1,6}\s*").unwrap();
        text = header.replace_all(&text, "").to_string();
        let bold_italic = Regex::new(r"\*\*\*|\*\*|__|\*|_").unwrap();
        text = bold_italic.replace_all(&text, "").to_string();
        let bq = Regex::new(r"^>\s*").unwrap();
        text = bq.replace_all(&text, "").to_string();
        let hr = Regex::new(r"^[\-\*_]{3,}\s*$").unwrap();
        text = hr.replace_all(&text, "").to_string();
        text.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_parse_text() {
        let doc = DocumentParser::parse_text("Hello world", "Test");
        assert_eq!(doc.title, "Test");
        assert_eq!(doc.content, "Hello world");
        assert!(doc.metadata.is_none());
    }
    #[test] fn test_parse_markdown_strips_syntax() {
        let md = r#"# Heading

This is **bold** and *italic*.

```rust
let x = 1;
```

[A link](https://example.com)
![An image](https://img.png)

> A quote
"#;
        let doc = DocumentParser::parse_markdown(md, "Md Doc");
        assert_eq!(doc.title, "Md Doc");
        assert!(!doc.content.contains("#"));
        assert!(!doc.content.contains("```"));
        assert!(!doc.content.contains("**"));
        assert!(doc.content.contains("bold"));
        assert!(doc.content.contains("italic"));
        assert!(doc.content.contains("A link"));
        assert!(doc.content.contains("An image"));
        assert!(doc.content.contains("A quote"));
    }
    #[test] fn test_parse_json() {
        let doc = DocumentParser::parse_json(r#"{"title": "My Doc", "content": "Hello"}"#).unwrap();
        assert_eq!(doc.title, "My Doc");
        assert_eq!(doc.content, "Hello");
    }
    #[test] fn test_parse_pdf_basic_extraction() {
        let pdf_bytes = b"%PDF-1.4
BT /F1 12 Tf 100 700 Td (Hello world) Tj ET
%%EOF";
        let doc = DocumentParser::parse_pdf(pdf_bytes, "Test PDF").unwrap();
        assert_eq!(doc.title, "Test PDF");
        assert!(doc.content.contains("Hello world"));
    }
    #[test] fn test_parse_pdf_no_text_fails() {
        let pdf_bytes = b"%PDF-1.4
1 0 obj
<< /Type /Catalog >>
endobj
%%EOF";
        assert!(DocumentParser::parse_pdf(pdf_bytes, "Empty PDF").is_err());
    }
}
