use crate::{
    assessment::{
        context::transcript_context_for_question,
        grader::{expected_completion_answer, is_completion_question},
    },
    models::JustificationResponse,
    AppResult, AppState,
};
use uuid::Uuid;

pub async fn justification_for_answer(
    state: &AppState,
    attempt_id: Uuid,
    answer_id: Uuid,
) -> AppResult<String> {
    if let Some(record) = sqlx::query_as::<_, (String,)>(
        "SELECT justification FROM answer_justifications WHERE attempt_answer_id = $1",
    )
    .bind(answer_id)
    .fetch_optional(&state.pool)
    .await?
    {
        return Ok(record.0);
    }

    let answer: crate::models::AttemptAnswerRecord =
        sqlx::query_as("SELECT * FROM attempt_answers WHERE id = $1 AND attempt_id = $2")
            .bind(answer_id)
            .bind(attempt_id)
            .fetch_one(&state.pool)
            .await?;

    let question: crate::models::QuestionRecord =
        sqlx::query_as("SELECT * FROM questions WHERE id = $1")
            .bind(answer.question_id)
            .fetch_one(&state.pool)
            .await?;

    let correct_answer = if question.question_type.eq_ignore_ascii_case("essay") {
        question
            .rubric
            .clone()
            .unwrap_or_else(|| "No rubric was stored.".to_string())
    } else if is_completion_question(&question.question_type) {
        expected_completion_answer(question.rubric.as_deref())
            .map(|answer| format!("Expected answer: {answer}"))
            .unwrap_or_else(|| "Correct answer not available.".to_string())
    } else {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT label, text FROM choices WHERE question_id = $1 AND is_correct = true",
        )
        .bind(question.id)
        .fetch_all(&state.pool)
        .await?;
        rows.first()
            .map(|(label, text)| format!("{label}: {text}"))
            .unwrap_or_else(|| "Correct answer not available.".to_string())
    };

    let transcript_context = transcript_context_for_question(state, &question).await?;
    let prompt = format!(
        "You are a helpful tutor explaining exam feedback to a student. Ground the explanation in the source video transcript context.\n\nQuestion: {}\nSource video transcript context:\n{}\nCorrect answer / rubric: {}\nStudent answered: {}\nScore given: {}/100\n\nIn 2-4 sentences:\n- Tell the student what they got right\n- Tell the student what they missed or got wrong\n- Give one concrete tip for improvement\nDo not repeat the question. Write directly to the student.",
        question.stem,
        transcript_context,
        correct_answer,
        answer.user_answer,
        answer.score.unwrap_or(0)
    );

    let justification = state.gemma.generate(&prompt).await?;

    sqlx::query(
        "INSERT INTO answer_justifications (attempt_answer_id, justification) VALUES ($1, $2)",
    )
    .bind(answer_id)
    .bind(&justification)
    .execute(&state.pool)
    .await?;

    Ok(justification)
}

pub async fn response_for_answer(
    state: &AppState,
    attempt_id: Uuid,
    answer_id: Uuid,
) -> AppResult<JustificationResponse> {
    let justification = justification_for_answer(state, attempt_id, answer_id).await?;
    Ok(JustificationResponse {
        answer_id,
        justification,
    })
}
