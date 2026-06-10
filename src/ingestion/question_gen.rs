use crate::{models::GeneratedQuestion, AppResult};

pub fn prompt_for_topic(topic_label: &str, chunk_text: &str, question_count: usize) -> String {
    format!(
        "You are creating educational questions for a learning platform.\n\nTopic: {topic_label}\nTranscript segment:\n{chunk_text}\n\nGenerate {question_count} questions. Return ONLY valid JSON array (no markdown):\n[\n  {{\n    \"stem\": \"question text\",\n    \"type\": \"mcq\" | \"true_false\" | \"essay\",\n    \"difficulty\": \"easy\" | \"medium\" | \"hard\",\n    \"rubric\": \"grading guidance (for essay only, else null)\",\n    \"choices\": [\n      {{ \"label\": \"A\", \"text\": \"...\", \"is_correct\": false }},\n      {{ \"label\": \"B\", \"text\": \"...\", \"is_correct\": true }}\n    ]\n  }}\n]"
    )
}

pub async fn generate_questions(
    client: &crate::llm::gemma::GemmaClient,
    topic_label: &str,
    chunk_text: &str,
    question_count: usize,
) -> AppResult<Vec<GeneratedQuestion>> {
    client.generate_json(&prompt_for_topic(topic_label, chunk_text, question_count)).await
}
