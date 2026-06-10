use crate::{models::TopicLabelResponse, AppResult};

pub fn prompt_for_chunk(chunk_text: &str) -> String {
    format!(
        "Given this video transcript segment, return ONLY valid JSON (no markdown).\n\nTranscript:\n{chunk_text}\n\nJSON schema:\n{{\n  \"label\": \"short topic name (3-6 words)\",\n  \"start_s\": <float>,\n  \"end_s\": <float>\n}}"
    )
}

pub async fn label_chunk(client: &crate::llm::gemma::GemmaClient, chunk_text: &str) -> AppResult<TopicLabelResponse> {
    client.generate_json(&prompt_for_chunk(chunk_text)).await
}
