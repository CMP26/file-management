use crate::{AppError, AppResult};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Clone)]
pub struct GemmaClient {
    base_url: String,
    model: String,
    client: Client,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    id: String,
}

impl GemmaClient {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: Client::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn list_model_ids(&self) -> AppResult<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .send()
            .await?
            .error_for_status()?;

        let parsed: ModelsResponse = response.json().await?;
        Ok(parsed.data.into_iter().map(|model| model.id).collect())
    }

    pub async fn generate(&self, prompt: &str) -> AppResult<String> {
        let payload = ChatCompletionRequest {
            model: &self.model,
            messages: vec![ChatMessage { role: "user", content: prompt }],
            temperature: Some(0.2),
            max_tokens: Some(2048),
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let parsed: ChatCompletionResponse = response.json().await?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AppError::external("llama.cpp returned no chat choices"))?
            .message
            .content;

        Ok(content)
    }

    pub async fn generate_json<T>(&self, prompt: &str) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        let mut last_error = None;

        for _attempt in 0..3 {
            let output = self.generate(prompt).await?;
            match serde_json::from_str::<T>(&output) {
                Ok(value) => return Ok(value),
                Err(error) => last_error = Some(error),
            }
        }

        Err(AppError::external(format!(
            "gemma did not return valid json after 3 attempts: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown parse failure".to_string())
        )))
    }
}
