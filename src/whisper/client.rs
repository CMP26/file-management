use crate::{models::TranscribeResponse, AppResult};
use reqwest::{multipart, Client};
use std::path::Path;

#[derive(Clone)]
pub struct WhisperClient {
    base_url: String,
    client: Client,
}

impl WhisperClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }

    pub async fn transcribe(
        &self,
        wav_path: &Path,
        language: &str,
    ) -> AppResult<TranscribeResponse> {
        let bytes = tokio::fs::read(wav_path).await?;
        let part = multipart::Part::bytes(bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")?;
        let form = multipart::Form::new()
            .part("file", part)
            .text("language", language.to_string());

        let response = self
            .client
            .post(format!("{}/transcribe", self.base_url))
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json::<TranscribeResponse>().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::WhisperClient;
    use tokio::{
        fs,
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use uuid::Uuid;

    #[test]
    fn trims_base_url() {
        let client = WhisperClient::new("http://localhost:8000///");

        assert_eq!(client.base_url, "http://localhost:8000");
    }

    #[tokio::test]
    async fn transcribes_audio_file_from_service_response() {
        let body = r#"{"full_text":"hello","segments":[{"start":0.0,"end":1.0,"text":"hello"}]}"#;
        let base_url = start_mock_server(json_response(body)).await;
        let client = WhisperClient::new(&base_url);
        let path = std::env::temp_dir().join(format!("nexalearn-whisper-{}.wav", Uuid::new_v4()));
        fs::write(&path, b"RIFF....WAVE").await.unwrap();

        let response = client.transcribe(&path, "en").await.unwrap();

        let _ = fs::remove_file(path).await;
        assert_eq!(response.full_text, "hello");
        assert_eq!(response.segments.len(), 1);
        assert_eq!(response.segments[0].text, "hello");
    }

    #[tokio::test]
    async fn missing_audio_file_returns_io_error() {
        let client = WhisperClient::new("http://127.0.0.1:1");
        let path = std::env::temp_dir().join(format!("missing-{}.wav", Uuid::new_v4()));

        let error = client.transcribe(&path, "en").await.unwrap_err();

        assert!(matches!(error, crate::AppError::Io(_)));
    }

    async fn start_mock_server(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 8192];
            let _ = stream.read(&mut buffer).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
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
