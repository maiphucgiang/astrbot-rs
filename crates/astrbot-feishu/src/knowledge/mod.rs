//! Knowledge base RAG data sources: Feishu Docs and Bitable

use reqwest::Method;
use serde_json::json;
use tracing::{debug, error, info};

use crate::{
    auth::FeishuAuth, BitableInfo, BitableRecord, DocumentInfo, FeishuError, PaginatedResponse,
    Result,
};

/// Trait for knowledge sources that can feed into RAG pipelines
#[async_trait::async_trait]
pub trait KnowledgeSource: Send + Sync {
    /// Retrieve raw text/markdown content for a given document
    async fn fetch_content(&self, document_id: &str) -> Result<String>;

    /// List available documents/tables
    async fn list_items(&self) -> Result<Vec<KnowledgeItem>>;
}

/// A generic knowledge item (doc or table)
#[derive(Clone, Debug)]
pub enum KnowledgeItem {
    Document(DocumentInfo),
    Bitable(BitableInfo),
}

/// Feishu Document client
pub struct DocClient {
    auth: FeishuAuth,
}

impl DocClient {
    pub fn new(auth: FeishuAuth) -> Self {
        Self { auth }
    }

    /// Get document metadata
    pub async fn get_doc(&self, document_id: &str) -> Result<DocumentInfo> {
        let path = format!("/docx/v1/documents/{}", document_id);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<DocumentInfo> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// Get document raw content (blocks)
    pub async fn get_doc_content(&self, document_id: &str) -> Result<String> {
        let path = format!("/docx/v1/documents/{}/raw_content", document_id);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        // Extract text from content field or blocks
        let data = api_resp.data.unwrap();
        let content = data
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// List blocks (for structured parsing)
    pub async fn list_blocks(
        &self,
        document_id: &str,
        page_token: Option<&str>,
        page_size: i32,
    ) -> Result<crate::PaginatedData<serde_json::Value>> {
        let mut path = format!(
            "/docx/v1/documents/{}/blocks?page_size={}",
            document_id, page_size
        );
        if let Some(token) = page_token {
            path.push_str(&format!("&page_token={}", token));
        }

        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: PaginatedResponse<serde_json::Value> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }
}

#[async_trait::async_trait]
impl KnowledgeSource for DocClient {
    async fn fetch_content(&self, document_id: &str) -> Result<String> {
        self.get_doc_content(document_id).await
    }

    async fn list_items(&self) -> Result<Vec<KnowledgeItem>> {
        // For docs, we don't have a global list API without folder context
        // Return empty for now, can be extended with folder search
        Ok(vec![])
    }
}

/// Feishu Bitable (multidimensional table) client
pub struct BitableClient {
    auth: FeishuAuth,
}

impl BitableClient {
    pub fn new(auth: FeishuAuth) -> Self {
        Self { auth }
    }

    /// Get bitable metadata
    pub async fn get_bitable(&self, app_token: &str) -> Result<BitableInfo> {
        let path = format!("/bitable/v1/apps/{}", app_token);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<BitableInfo> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// List tables in a bitable
    pub async fn list_tables(
        &self,
        app_token: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let path = format!("/bitable/v1/apps/{}/tables", app_token);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let items = data
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(items)
    }

    /// List records in a table
    pub async fn list_records(
        &self,
        app_token: &str,
        table_id: &str,
        page_token: Option<&str>,
        page_size: i32,
    ) -> Result<crate::PaginatedData<BitableRecord>> {
        let mut path = format!(
            "/bitable/v1/apps/{}/tables/{}/records?page_size={}",
            app_token, table_id, page_size
        );
        if let Some(token) = page_token {
            path.push_str(&format!("&page_token={}", token));
        }

        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: PaginatedResponse<BitableRecord> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// Create a record
    pub async fn create_record(
        &self,
        app_token: &str,
        table_id: &str,
        fields: serde_json::Value,
    ) -> Result<BitableRecord> {
        let path = format!(
            "/bitable/v1/apps/{}/tables/{}/records",
            app_token, table_id
        );
        let body = json!({ "fields": fields });

        let req = self.auth.auth_request(Method::POST, &path).await?;
        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<BitableRecord> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// Update a record
    pub async fn update_record(
        &self,
        app_token: &str,
        table_id: &str,
        record_id: &str,
        fields: serde_json::Value,
    ) -> Result<BitableRecord> {
        let path = format!(
            "/bitable/v1/apps/{}/tables/{}/records/{}",
            app_token, table_id, record_id
        );
        let body = json!({ "fields": fields });

        let req = self.auth.auth_request(Method::PUT, &path).await?;
        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<BitableRecord> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// Search records with filter
    pub async fn search_records(
        &self,
        app_token: &str,
        table_id: &str,
        filter: serde_json::Value,
    ) -> Result<Vec<BitableRecord>> {
        let path = format!(
            "/bitable/v1/apps/{}/tables/{}/records/search",
            app_token, table_id
        );
        let body = json!({ "filter": filter });

        let req = self.auth.auth_request(Method::POST, &path).await?;
        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let items = data
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let records: Vec<BitableRecord> = items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();

        Ok(records)
    }

    /// Convert records to markdown for RAG context
    pub async fn records_to_markdown(
        &self,
        app_token: &str,
        table_id: &str,
    ) -> Result<String> {
        let mut all_records = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let page = self
                .list_records(
                    app_token,
                    table_id,
                    page_token.as_deref(),
                    500,
                )
                .await?;

            all_records.extend(page.items);

            if !page.has_more || page.page_token.is_none() {
                break;
            }
            page_token = page.page_token;
        }

        let mut md = String::new();
        md.push_str(&format!("# Bitable Records ({} total)\n\n", all_records.len()));

        for (i, record) in all_records.iter().enumerate() {
            md.push_str(&format!("## Record {} (ID: {})\n\n", i + 1, record.record_id));
            md.push_str(&format!("```json\n{}\n```\n\n", record.fields.to_string()));
        }

        info!(
            "Exported {} records from bitable {}/{}",
            all_records.len(),
            app_token,
            table_id
        );
        Ok(md)
    }
}

#[async_trait::async_trait]
impl KnowledgeSource for BitableClient {
    async fn fetch_content(&self, document_id: &str) -> Result<String> {
        // document_id format: "app_token:table_id"
        let parts: Vec<&str> = document_id.split(':').collect();
        if parts.len() != 2 {
            return Err(FeishuError::Config(
                "Bitable document_id must be 'app_token:table_id'".into(),
            ));
        }
        self.records_to_markdown(parts[0], parts[1]).await
    }

    async fn list_items(&self) -> Result<Vec<KnowledgeItem>> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_item_doc() {
        let doc = DocumentInfo {
            document_id: "doc_xxx".into(),
            title: Some("Test".into()),
            url: None,
            create_time: None,
            update_time: None,
        };
        let item = KnowledgeItem::Document(doc);
        assert!(matches!(item, KnowledgeItem::Document(_)));
    }
}
