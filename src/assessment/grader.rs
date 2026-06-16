use crate::{
    assessment::context::transcript_context_for_question,
    models::{GradeResponse, QuestionRecord},
    AppResult, AppState,
};
use uuid::Uuid;

pub async fn grade_answer(
    state: &AppState,
    question_id: Uuid,
    user_answer: &str,
) -> AppResult<GradeResponse> {
    let question: QuestionRecord = sqlx::query_as("SELECT * FROM questions WHERE id = $1")
        .bind(question_id)
        .fetch_one(&state.pool)
        .await?;

    if question.question_type.eq_ignore_ascii_case("essay") {
        let transcript_context = transcript_context_for_question(state, &question).await?;
        let prompt = format!(
            "You are a strict academic grader. Use the source video transcript context when judging factual accuracy. Return ONLY valid JSON (no markdown).\n\nQuestion: {}\nSource video transcript context:\n{}\nGrading rubric: {}\nStudent answer: {}\n\nJSON schema:\n{{\n  \"score\": <integer 0-100>,\n  \"is_correct\": <boolean, true if score >= 60>\n}}",
            question.stem,
            transcript_context,
            question.rubric.clone().unwrap_or_default(),
            user_answer
        );
        let result: GradeResponse = state.gemma.generate_json(&prompt).await?;
        return Ok(result);
    }

    let correct_choice: Option<(String,)> = sqlx::query_as(
        "SELECT label FROM choices WHERE question_id = $1 AND is_correct = true LIMIT 1",
    )
    .bind(question_id)
    .fetch_optional(&state.pool)
    .await?;

    let is_correct = correct_choice
        .as_ref()
        .map(|(label,)| label.eq_ignore_ascii_case(user_answer.trim()))
        .unwrap_or(false);

    Ok(GradeResponse {
        score: if is_correct { 100 } else { 0 },
        is_correct,
    })
}
