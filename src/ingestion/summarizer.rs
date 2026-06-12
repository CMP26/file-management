use crate::AppResult;

pub fn prompt_for_summary(full_text: &str) -> String {
    format!(
        "Summarize the following video transcript for a student in 1-2 short paragraphs maximum.\nKeep the whole summary under 120 words.\nFocus only on the key concepts, not timestamps.\nReturn only the summary text, with no heading or bullet list.\n\nTranscript:\n{full_text}"
    )
}

pub async fn summarize(
    client: &crate::llm::gemma::GemmaClient,
    full_text: &str,
) -> AppResult<String> {
    client.generate(&prompt_for_summary(full_text)).await
}
