use crate::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct OllamaEmbeddingClient {
    base_url: String,
    model: String,
    client: Client,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbeddingClient {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: Client::new(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn embed(&self, input: &str) -> AppResult<Vec<f32>> {
        self.embed_text(input).await
    }

    pub async fn embed_query(&self, input: &str) -> AppResult<Vec<f32>> {
        self.embed_text(&format!("search_query: {input}")).await
    }

    pub async fn embed_document(&self, input: &str) -> AppResult<Vec<f32>> {
        self.embed_text(&format!("search_document: {input}")).await
    }

    async fn embed_text(&self, input: &str) -> AppResult<Vec<f32>> {
        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&EmbedRequest {
                model: &self.model,
                input,
            })
            .send()
            .await?
            .error_for_status()?;
        let parsed: EmbedResponse = response.json().await?;
        parsed
            .embeddings
            .into_iter()
            .next()
            .filter(|embedding| !embedding.is_empty())
            .ok_or_else(|| AppError::external("ollama returned no embedding"))
    }
}
