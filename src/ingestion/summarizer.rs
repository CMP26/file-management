use crate::AppResult;

pub fn prompt_for_summary(full_text: &str) -> String {
    format!(
        "Summarize the following video transcript for a student in 3-5 clear paragraphs.\nFocus on key concepts, not timestamps.\n\nTranscript:\n{full_text}"
    )
}

pub async fn summarize(client: &crate::llm::gemma::GemmaClient, full_text: &str) -> AppResult<String> {
    client.generate(&prompt_for_summary(full_text)).await
}
