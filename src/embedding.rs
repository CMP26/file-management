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

#[cfg(test)]
mod tests {
    use super::OllamaEmbeddingClient;
    use crate::AppError;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn stores_model_and_trims_base_url() {
        let client = OllamaEmbeddingClient::new("http://localhost:11434///", "nomic");

        assert_eq!(client.model(), "nomic");
        assert_eq!(client.base_url, "http://localhost:11434");
    }

    #[tokio::test]
    async fn embeds_plain_query_and_document_inputs() {
        let base_url = start_embed_server(3).await;
        let client = OllamaEmbeddingClient::new(&base_url, "nomic");

        assert_eq!(client.embed("hello").await.unwrap(), vec![0.1, 0.2]);
        assert_eq!(client.embed_query("hello").await.unwrap(), vec![0.1, 0.2]);
        assert_eq!(
            client.embed_document("hello").await.unwrap(),
            vec![0.1, 0.2]
        );
    }

    #[tokio::test]
    async fn rejects_empty_embedding_response() {
        let base_url = start_mock_server(vec![json_response(r#"{"embeddings":[]}"#)]).await;
        let client = OllamaEmbeddingClient::new(&base_url, "nomic");

        let error = client
            .embed("hello")
            .await
            .expect_err("empty embedding fails");

        assert!(matches!(error, AppError::External(_)));
    }

    async fn start_embed_server(request_count: usize) -> String {
        start_mock_server(
            (0..request_count)
                .map(|_| json_response(r#"{"embeddings":[[0.1,0.2]]}"#))
                .collect(),
        )
        .await
    }

    async fn start_mock_server(responses: Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer).await.unwrap();
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}")
    }

    fn json_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        )
    }
}
