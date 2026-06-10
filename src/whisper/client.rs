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

    pub async fn transcribe(&self, wav_path: &Path, language: &str) -> AppResult<TranscribeResponse> {
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
