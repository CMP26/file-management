use crate::{AppError, AppResult};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{sync::Semaphore, time::sleep};
use tracing::info;

const GENERATE_ATTEMPTS: usize = 3;
static GEMMA_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct GemmaClient {
    base_url: String,
    model: String,
    client: Client,
    request_limiter: Arc<Semaphore>,
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
    pub fn new(
        base_url: &str,
        model: &str,
        max_concurrent_requests: usize,
        request_timeout_seconds: u64,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(request_timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client,
            request_limiter: Arc::new(Semaphore::new(max_concurrent_requests)),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn list_model_ids(&self) -> AppResult<Vec<String>> {
        info!(
            base_url = %self.base_url,
            "gemma models request started"
        );
        let response = self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .send()
            .await?
            .error_for_status()?;

        let parsed: ModelsResponse = response.json().await?;
        let model_ids = parsed
            .data
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        info!(
            model_count = model_ids.len(),
            "gemma models request completed"
        );
        Ok(model_ids)
    }

    pub async fn generate(&self, prompt: &str) -> AppResult<String> {
        let request_id = GEMMA_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let prompt_chars = prompt.chars().count();
        info!(
            request_id,
            model = %self.model,
            prompt_chars,
            max_attempts = GENERATE_ATTEMPTS,
            available_permits = self.request_limiter.available_permits(),
            "gemma generation queued"
        );

        let payload = ChatCompletionRequest {
            model: &self.model,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
            temperature: Some(0.2),
            max_tokens: Some(2048),
            stream: false,
        };

        let mut last_error = None;
        for attempt in 1..=GENERATE_ATTEMPTS {
            match self.generate_once(request_id, attempt, &payload).await {
                Ok(content) => {
                    info!(
                        request_id,
                        attempt,
                        response_chars = content.chars().count(),
                        "gemma generation completed"
                    );
                    return Ok(content);
                }
                Err(error) => {
                    info!(
                        request_id,
                        attempt,
                        error = %error,
                        will_retry = attempt < GENERATE_ATTEMPTS,
                        "gemma generation attempt failed"
                    );
                    last_error = Some(error);
                    if attempt < GENERATE_ATTEMPTS {
                        sleep(Duration::from_millis(400 * attempt as u64)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| AppError::external("gemma generation failed")))
    }

    async fn generate_once(
        &self,
        request_id: u64,
        attempt: usize,
        payload: &ChatCompletionRequest<'_>,
    ) -> AppResult<String> {
        info!(
            request_id,
            attempt,
            available_permits = self.request_limiter.available_permits(),
            "gemma waiting for request permit"
        );
        let _permit = self
            .request_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::external("gemma request queue was closed"))?;

        info!(request_id, attempt, "gemma request permit acquired");
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        info!(
            request_id,
            attempt,
            status = %status,
            "gemma http response received"
        );
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(AppError::external(format!(
                "gemma returned retryable status {status}"
            )));
        }
        let response = response.error_for_status()?;

        let parsed: ChatCompletionResponse = response.json().await?;
        info!(
            request_id,
            attempt,
            choice_count = parsed.choices.len(),
            "gemma response parsed"
        );
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

        for attempt in 1..=3 {
            info!(
                attempt,
                max_attempts = 3,
                "gemma json generation attempt started"
            );
            let output = self.generate(prompt).await?;
            match parse_json_output::<T>(&output) {
                Ok(value) => {
                    info!(attempt, "gemma json generation parsed successfully");
                    return Ok(value);
                }
                Err(error) => {
                    info!(
                        attempt,
                        error = %error,
                        "gemma json generation parse failed"
                    );
                    last_error = Some(error);
                }
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

fn parse_json_output<T>(output: &str) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    let trimmed = output.trim();
    if let Ok(value) = serde_json::from_str::<T>(trimmed) {
        return Ok(value);
    }

    let without_fence = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);

    if let Ok(value) = serde_json::from_str::<T>(without_fence) {
        return Ok(value);
    }

    let first_array = without_fence.find('[');
    let first_object = without_fence.find('{');
    let start = match (first_array, first_object) {
        (Some(array), Some(object)) => Some(array.min(object)),
        (Some(array), None) => Some(array),
        (None, Some(object)) => Some(object),
        (None, None) => None,
    };

    if let Some(start) = start {
        let end = without_fence
            .rfind(']')
            .into_iter()
            .chain(without_fence.rfind('}'))
            .max();

        if let Some(end) = end {
            if end > start {
                return serde_json::from_str::<T>(&without_fence[start..=end]);
            }
        }
    }

    serde_json::from_str::<T>(trimmed)
}

#[cfg(test)]
mod tests {
    use super::{parse_json_output, GemmaClient};
    use serde::Deserialize;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[derive(Debug, Deserialize, PartialEq)]
    struct ParsedValue {
        score: i32,
    }

    #[test]
    fn parses_plain_fenced_and_embedded_json() {
        assert_eq!(
            parse_json_output::<ParsedValue>(r#"{"score": 7}"#).unwrap(),
            ParsedValue { score: 7 }
        );
        assert_eq!(
            parse_json_output::<ParsedValue>("```json\n{\"score\": 8}\n```").unwrap(),
            ParsedValue { score: 8 }
        );
        assert_eq!(
            parse_json_output::<ParsedValue>("Answer:\n{\"score\": 9}\nThanks").unwrap(),
            ParsedValue { score: 9 }
        );
    }

    #[test]
    fn rejects_output_without_json() {
        assert!(parse_json_output::<ParsedValue>("no json here").is_err());
    }

    #[test]
    fn client_trims_base_url_and_keeps_model() {
        let client = GemmaClient::new("http://localhost:8100///", "gemma-test", 1, 5);

        assert_eq!(client.base_url(), "http://localhost:8100");
        assert_eq!(client.model(), "gemma-test");
    }

    #[tokio::test]
    async fn lists_model_ids_from_openai_compatible_endpoint() {
        let base_url = start_mock_server(vec![json_response(
            r#"{"data":[{"id":"gemma"},{"id":"other"}]}"#,
        )])
        .await;
        let client = GemmaClient::new(&base_url, "gemma", 1, 5);

        let models = client.list_model_ids().await.unwrap();

        assert_eq!(models, vec!["gemma".to_string(), "other".to_string()]);
    }

    #[tokio::test]
    async fn generates_text_from_chat_completion_endpoint() {
        let base_url = start_mock_server(vec![json_response(
            r#"{"choices":[{"message":{"content":" answer "}}]}"#,
        )])
        .await;
        let client = GemmaClient::new(&base_url, "gemma", 1, 5);

        let output = client.generate("Say hi").await.unwrap();

        assert_eq!(output, " answer ");
    }

    #[tokio::test]
    async fn generate_json_parses_model_response() {
        let base_url = start_mock_server(vec![json_response(
            r#"{"choices":[{"message":{"content":"```json\n{\"score\":42}\n```"}}]}"#,
        )])
        .await;
        let client = GemmaClient::new(&base_url, "gemma", 1, 5);

        let parsed: ParsedValue = client.generate_json("grade").await.unwrap();

        assert_eq!(parsed, ParsedValue { score: 42 });
    }

    #[tokio::test]
    async fn retryable_status_is_retried() {
        let base_url = start_mock_server(vec![
            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\n\r\n".to_string(),
            json_response(r#"{"choices":[{"message":{"content":"ok"}}]}"#),
        ])
        .await;
        let client = GemmaClient::new(&base_url, "gemma", 1, 5);

        let output = client.generate("retry").await.unwrap();

        assert_eq!(output, "ok");
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
