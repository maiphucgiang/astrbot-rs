use super::document::{Document, TextChunk};

#[derive(Debug, Clone)]
pub enum SplitStrategy {
    FixedSize { chunk_size: usize, overlap: usize },
    Paragraph,
    Recursive { chunk_size: usize, overlap: usize },
}

#[derive(Debug, Clone)]
pub struct TextSplitter {
    strategy: SplitStrategy,
}

impl TextSplitter {
    pub fn new(strategy: SplitStrategy) -> Self {
        Self { strategy }
    }
    pub fn split(&self, doc: &Document) -> Vec<TextChunk> {
        match &self.strategy {
            SplitStrategy::FixedSize {
                chunk_size,
                overlap,
            } => self.split_fixed(&doc.id, &doc.content, *chunk_size, *overlap),
            SplitStrategy::Paragraph => self.split_paragraph(&doc.id, &doc.content),
            SplitStrategy::Recursive {
                chunk_size,
                overlap,
            } => self.split_recursive(&doc.id, &doc.content, *chunk_size, *overlap),
        }
    }
    fn split_fixed(
        &self,
        doc_id: &str,
        content: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Vec<TextChunk> {
        if content.is_empty() {
            return Vec::new();
        }
        let mut chunks = Vec::new();
        let mut start = 0;
        let mut index = 0;
        while start < content.len() {
            let end = (start + chunk_size).min(content.len());
            let text = content[start..end].to_string();
            chunks.push(TextChunk {
                id: format!("{}-chunk-{}", doc_id, index),
                doc_id: doc_id.to_string(),
                text,
                index,
                metadata: None,
            });
            let advance = chunk_size.saturating_sub(overlap);
            if advance == 0 {
                break;
            }
            start += advance;
            if start >= content.len() && start < end {
                break;
            }
            index += 1;
        }
        chunks
    }
    fn split_paragraph(&self, doc_id: &str, content: &str) -> Vec<TextChunk> {
        content
            .split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .enumerate()
            .map(|(index, text)| TextChunk {
                id: format!("{}-chunk-{}", doc_id, index),
                doc_id: doc_id.to_string(),
                text: text.trim().to_string(),
                index,
                metadata: None,
            })
            .collect()
    }
    fn split_recursive(
        &self,
        doc_id: &str,
        content: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Vec<TextChunk> {
        let paragraphs: Vec<&str> = content
            .split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .collect();
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut index = 0;
        for para in paragraphs {
            if current.len() + para.len() > chunk_size && !current.is_empty() {
                chunks.push(TextChunk {
                    id: format!("{}-chunk-{}", doc_id, index),
                    doc_id: doc_id.to_string(),
                    text: current.trim().to_string(),
                    index,
                    metadata: None,
                });
                index += 1;
                current = String::new();
            }
            current.push_str(para);
            current.push('\n');
        }
        if !current.is_empty() {
            if current.len() > chunk_size {
                chunks.extend(self.split_fixed(doc_id, &current, chunk_size, overlap));
            } else {
                chunks.push(TextChunk {
                    id: format!("{}-chunk-{}", doc_id, index),
                    doc_id: doc_id.to_string(),
                    text: current.trim().to_string(),
                    index,
                    metadata: None,
                });
            }
        }
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_doc(content: &str) -> Document {
        Document {
            id: "doc-1".to_string(),
            title: "Test".to_string(),
            content: content.to_string(),
            metadata: None,
        }
    }
    #[test]
    fn test_fixed_size_split() {
        let splitter = TextSplitter::new(SplitStrategy::FixedSize {
            chunk_size: 10,
            overlap: 2,
        });
        let chunks = splitter.split(&make_doc("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].text, "ABCDEFGHIJ");
        assert_eq!(chunks[1].text, "IJKLMNOPQR");
    }
    #[test]
    fn test_paragraph_split() {
        let splitter = TextSplitter::new(SplitStrategy::Paragraph);
        let chunks = splitter.split(&make_doc("Para 1.\n\nPara 2.\n\nPara 3."));
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "Para 1.");
        assert_eq!(chunks[1].text, "Para 2.");
        assert_eq!(chunks[2].text, "Para 3.");
    }
    #[test]
    fn test_empty_content() {
        let splitter = TextSplitter::new(SplitStrategy::FixedSize {
            chunk_size: 10,
            overlap: 2,
        });
        assert!(splitter.split(&make_doc("")).is_empty());
    }
}
