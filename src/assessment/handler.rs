use crate::{
    assessment::{grader::grade_answer, justifier::response_for_answer},
    models::{
        AttemptBreakdownItem, ChoiceRecord, ExamAttemptRecord, JustificationResponse, QuestionChoiceResponse,
        QuestionRecord, QuestionsByVideoResponse, StartExamRequest, StartExamResponse, SubmitAttemptRequest,
        SubmitAttemptResponse, TopicQuestionGroupResponse,
    },
    AppResult, AppState,
};
use axum::{extract::{Path, Query, State}, Json};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Default, serde::Deserialize)]
pub struct QuestionFilters {
    pub topic_id: Option<Uuid>,
    pub r#type: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/videos/{video_id}/questions",
    tag = "Assessment",
    params(
        ("video_id" = Uuid, Path, description = "Video id"),
        ("topic_id" = Option<Uuid>, Query, description = "Filter by topic id"),
        ("type" = Option<String>, Query, description = "Filter by question type")
    ),
    responses(
        (status = 200, description = "Questions grouped by topic", body = QuestionsByVideoResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_video_questions(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
    Query(filters): Query<QuestionFilters>,
) -> AppResult<Json<QuestionsByVideoResponse>> {
    let topics: Vec<(Uuid, String)> = match filters.topic_id {
        Some(topic_id) => sqlx::query_as("SELECT id, label FROM topics WHERE video_id = $1 AND id = $2 ORDER BY seq_order")
            .bind(video_id)
            .bind(topic_id)
            .fetch_all(&state.pool)
            .await?,
        None => sqlx::query_as("SELECT id, label FROM topics WHERE video_id = $1 ORDER BY seq_order")
            .bind(video_id)
            .fetch_all(&state.pool)
            .await?,
    };

    let questions: Vec<QuestionRecord> = match &filters.r#type {
        Some(question_type) => sqlx::query_as("SELECT * FROM questions WHERE video_id = $1 AND question_type = $2 ORDER BY created_at")
            .bind(video_id)
            .bind(question_type)
            .fetch_all(&state.pool)
            .await?,
        None => sqlx::query_as("SELECT * FROM questions WHERE video_id = $1 ORDER BY created_at")
            .bind(video_id)
            .fetch_all(&state.pool)
            .await?,
    };

    let mut question_map: HashMap<Uuid, Vec<QuestionChoiceResponse>> = HashMap::new();
    let question_ids: Vec<Uuid> = questions.iter().map(|question| question.id).collect();

    if !question_ids.is_empty() {
        let choice_rows: Vec<ChoiceRecord> = sqlx::query_as(
            "SELECT * FROM choices WHERE question_id = ANY($1) ORDER BY label",
        )
        .bind(&question_ids)
        .fetch_all(&state.pool)
        .await?;

        for choice in choice_rows {
            question_map.entry(choice.question_id).or_default().push(QuestionChoiceResponse {
                label: choice.label,
                text: choice.text,
            });
        }
    }

    let mut grouped_topics = Vec::new();
    for (topic_id, label) in topics {
        let topic_questions = questions
            .iter()
            .filter(|question| question.topic_id == Some(topic_id))
            .map(|question| crate::models::QuestionResponse {
                id: question.id,
                stem: question.stem.clone(),
                question_type: question.question_type.clone(),
                difficulty: question.difficulty.clone(),
                choices: question_map.get(&question.id).cloned().unwrap_or_default(),
            })
            .collect();

        grouped_topics.push(TopicQuestionGroupResponse {
            topic_id,
            label,
            questions: topic_questions,
        });
    }

    Ok(Json(QuestionsByVideoResponse { video_id, topics: grouped_topics }))
}

#[utoipa::path(
    post,
    path = "/api/videos/{video_id}/exams/start",
    tag = "Assessment",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    request_body = StartExamRequest,
    responses(
        (status = 200, description = "Exam attempt started", body = StartExamResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn start_exam_attempt(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
    Json(payload): Json<StartExamRequest>,
) -> AppResult<Json<StartExamResponse>> {
    let attempt_id: Uuid = sqlx::query_scalar(
        "INSERT INTO exam_attempts (user_id, video_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(payload.user_id)
    .bind(video_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(StartExamResponse { attempt_id }))
}

#[utoipa::path(
    post,
    path = "/api/exams/{attempt_id}/submit",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id")
    ),
    request_body = SubmitAttemptRequest,
    responses(
        (status = 200, description = "Attempt submitted and graded", body = SubmitAttemptResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn submit_attempt(
    State(state): State<AppState>,
    Path(attempt_id): Path<Uuid>,
    Json(payload): Json<SubmitAttemptRequest>,
) -> AppResult<Json<SubmitAttemptResponse>> {
    let attempt: ExamAttemptRecord = sqlx::query_as("SELECT * FROM exam_attempts WHERE id = $1")
        .bind(attempt_id)
        .fetch_one(&state.pool)
        .await?;

    let mut total_score = 0i32;
    let mut breakdown = Vec::new();

    for answer_input in payload.answers {
        let grade = grade_answer(&state, answer_input.question_id, &answer_input.user_answer).await?;
        let answer_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO attempt_answers (attempt_id, question_id, user_answer, is_correct, score, graded_at)
            VALUES ($1, $2, $3, $4, $5, now())
            RETURNING id
            "#,
        )
        .bind(attempt_id)
        .bind(answer_input.question_id)
        .bind(&answer_input.user_answer)
        .bind(grade.is_correct)
        .bind(grade.score)
        .fetch_one(&state.pool)
        .await?;

        total_score += i32::from(grade.score);
        breakdown.push(AttemptBreakdownItem {
            answer_id,
            question_id: answer_input.question_id,
            is_correct: grade.is_correct,
            score: grade.score,
        });
    }

    sqlx::query("UPDATE exam_attempts SET submitted_at = now() WHERE id = $1")
        .bind(attempt.id)
        .execute(&state.pool)
        .await?;

    Ok(Json(SubmitAttemptResponse {
        attempt_id,
        total_score,
        breakdown,
    }))
}

#[utoipa::path(
    get,
    path = "/api/exams/{attempt_id}/answers/{answer_id}/justification",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id"),
        ("answer_id" = Uuid, Path, description = "Answer id")
    ),
    responses(
        (status = 200, description = "Model-generated justification", body = JustificationResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_justification(
    State(state): State<AppState>,
    Path((attempt_id, answer_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<JustificationResponse>> {
    Ok(Json(response_for_answer(&state, attempt_id, answer_id).await?))
}
