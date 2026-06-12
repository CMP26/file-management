use crate::{models::GeneratedQuestion, AppResult};

pub fn prompt_for_essay_questions(
    topic_context: &str,
    transcript_text: &str,
    question_count: usize,
) -> String {
    format!(
        "You are creating an assessment for a learning platform.\n\nTopics:\n{topic_context}\n\nTranscript:\n{transcript_text}\n\nGenerate exactly {question_count} essay questions total: one essay question for each topic, in the same order as the Topics list. Return ONLY valid JSON array, with no markdown and no extra text. Every item must use this exact schema:\n[\n  {{\n    \"stem\": \"essay question text\",\n    \"question_type\": \"essay\",\n    \"difficulty\": \"medium\",\n    \"rubric\": \"specific grading guidance for this essay question\",\n    \"choices\": null\n  }}\n]\n\nRules:\n- Generate only essay questions.\n- Do not generate MCQ or true/false questions.\n- Return exactly {question_count} items.\n- Each question should assess its matching topic."
    )
}

pub async fn generate_essay_questions(
    client: &crate::llm::gemma::GemmaClient,
    topic_context: &str,
    transcript_text: &str,
    question_count: usize,
) -> AppResult<Vec<GeneratedQuestion>> {
    client
        .generate_json(&prompt_for_essay_questions(
            topic_context,
            transcript_text,
            question_count,
        ))
        .await
}
