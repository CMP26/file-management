use crate::AppResult;
use serde::Deserialize;

pub fn prompt_for_chunk(chunk_text: &str) -> String {
    format!(
        "Given this video transcript segment, create a short topic name. Return ONLY valid JSON, with no markdown and no extra text.\n\nTranscript:\n{chunk_text}\n\nJSON schema:\n{{\n  \"label\": \"short topic name (3-6 words)\"\n}}"
    )
}

#[derive(Debug, Deserialize)]
struct TopicLabelOnly {
    label: String,
}

pub async fn label_chunk(
    client: &crate::llm::gemma::GemmaClient,
    chunk_text: &str,
) -> AppResult<String> {
    let response: TopicLabelOnly = client.generate_json(&prompt_for_chunk(chunk_text)).await?;
    Ok(response.label)
}
