use crate::{
    assessment::{grader::grade_answer, justifier::response_for_answer},
    models::{
        AttemptBreakdownItem, ChoiceRecord, CourseRandomQuestionResponse,
        CourseRandomQuestionsResponse, ExamAttemptRecord, JustificationResponse,
        QuestionChoiceResponse, QuestionRecord, QuestionsByVideoResponse, SourceVideoResponse,
        StartExamRequest, StartExamResponse, SubmitAttemptRequest, SubmitAttemptResponse,
        TopicQuestionGroupResponse,
    },
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Default, serde::Deserialize)]
pub struct QuestionFilters {
    pub topic_id: Option<Uuid>,
    pub r#type: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RandomQuestionFilters {
    pub count: Option<i64>,
    pub r#type: Option<String>,
}

#[derive(Debug, FromRow)]
struct CourseRandomQuestionRow {
    id: Uuid,
    video_id: Uuid,
    topic_id: Option<Uuid>,
    stem: String,
    question_type: String,
    difficulty: Option<String>,
    source_video_title: String,
    topic_label: Option<String>,
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
        Some(topic_id) => {
            sqlx::query_as(
                "SELECT id, label FROM topics WHERE video_id = $1 AND id = $2 ORDER BY seq_order",
            )
            .bind(video_id)
            .bind(topic_id)
            .fetch_all(&state.pool)
            .await?
        }
        None => {
            sqlx::query_as("SELECT id, label FROM topics WHERE video_id = $1 ORDER BY seq_order")
                .bind(video_id)
                .fetch_all(&state.pool)
                .await?
        }
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

    let question_ids: Vec<Uuid> = questions.iter().map(|question| question.id).collect();
    let question_map = load_choice_map(&state, &question_ids).await?;

    let mut grouped_topics = Vec::new();
    for (topic_id, label) in topics {
        let topic_questions = questions
            .iter()
            .filter(|question| question.topic_id == Some(topic_id))
            .map(|question| crate::models::QuestionResponse {
                id: question.id,
                video_id: question.video_id,
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

    Ok(Json(QuestionsByVideoResponse {
        video_id,
        topics: grouped_topics,
    }))
}

#[utoipa::path(
    get,
    path = "/api/courses/{course_id}/questions/random",
    tag = "Assessment",
    params(
        ("course_id" = Uuid, Path, description = "Course id"),
        ("count" = Option<i64>, Query, description = "Number of random questions to return, default 10, max 100"),
        ("type" = Option<String>, Query, description = "Filter by question type")
    ),
    responses(
        (status = 200, description = "Random questions from videos in a course, including source video metadata", body = CourseRandomQuestionsResponse),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Course not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_course_random_questions(
    State(state): State<AppState>,
    Path(course_id): Path<Uuid>,
    Query(filters): Query<RandomQuestionFilters>,
) -> AppResult<Json<CourseRandomQuestionsResponse>> {
    ensure_course_exists(&state, course_id).await?;

    let count = filters.count.unwrap_or(10);
    if !(1..=100).contains(&count) {
        return Err(AppError::bad_request(
            "count must be between 1 and 100 random questions",
        ));
    }

    let rows: Vec<CourseRandomQuestionRow> = match &filters.r#type {
        Some(question_type) => {
            sqlx::query_as(
                r#"
            SELECT
                q.id,
                q.video_id,
                q.topic_id,
                q.stem,
                q.question_type,
                q.difficulty,
                v.title AS source_video_title,
                t.label AS topic_label
            FROM questions q
            JOIN videos v ON v.id = q.video_id
            LEFT JOIN topics t ON t.id = q.topic_id
            WHERE v.course_id = $1 AND q.question_type = $2
            ORDER BY random()
            LIMIT $3
            "#,
            )
            .bind(course_id)
            .bind(question_type)
            .bind(count)
            .fetch_all(&state.pool)
            .await?
        }
        None => {
            sqlx::query_as(
                r#"
            SELECT
                q.id,
                q.video_id,
                q.topic_id,
                q.stem,
                q.question_type,
                q.difficulty,
                v.title AS source_video_title,
                t.label AS topic_label
            FROM questions q
            JOIN videos v ON v.id = q.video_id
            LEFT JOIN topics t ON t.id = q.topic_id
            WHERE v.course_id = $1
            ORDER BY random()
            LIMIT $2
            "#,
            )
            .bind(course_id)
            .bind(count)
            .fetch_all(&state.pool)
            .await?
        }
    };

    let question_ids: Vec<Uuid> = rows.iter().map(|question| question.id).collect();
    let choice_map = load_choice_map(&state, &question_ids).await?;

    let questions = rows
        .into_iter()
        .map(|row| CourseRandomQuestionResponse {
            id: row.id,
            source_video: SourceVideoResponse {
                id: row.video_id,
                title: row.source_video_title,
            },
            topic_id: row.topic_id,
            topic_label: row.topic_label,
            stem: row.stem,
            question_type: row.question_type,
            difficulty: row.difficulty,
            choices: choice_map.get(&row.id).cloned().unwrap_or_default(),
        })
        .collect();

    Ok(Json(CourseRandomQuestionsResponse {
        course_id,
        requested_count: count,
        questions,
    }))
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
        (status = 400, description = "Question does not belong to the attempt video"),
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
        let question_video_id: Uuid =
            sqlx::query_scalar("SELECT video_id FROM questions WHERE id = $1")
                .bind(answer_input.question_id)
                .fetch_one(&state.pool)
                .await?;
        if question_video_id != attempt.video_id {
            return Err(AppError::bad_request(format!(
                "question {} does not belong to video {}",
                answer_input.question_id, attempt.video_id
            )));
        }

        let grade =
            grade_answer(&state, answer_input.question_id, &answer_input.user_answer).await?;
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
    Ok(Json(
        response_for_answer(&state, attempt_id, answer_id).await?,
    ))
}

async fn ensure_course_exists(state: &AppState, course_id: Uuid) -> AppResult<()> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM courses WHERE id = $1)")
            .bind(course_id)
            .fetch_one(&state.pool)
            .await?;

    if exists {
        Ok(())
    } else {
        Err(AppError::not_found(format!(
            "course {course_id} was not found"
        )))
    }
}

async fn load_choice_map(
    state: &AppState,
    question_ids: &[Uuid],
) -> AppResult<HashMap<Uuid, Vec<QuestionChoiceResponse>>> {
    let mut question_map: HashMap<Uuid, Vec<QuestionChoiceResponse>> = HashMap::new();
    if question_ids.is_empty() {
        return Ok(question_map);
    }

    let choice_rows: Vec<ChoiceRecord> =
        sqlx::query_as("SELECT * FROM choices WHERE question_id = ANY($1) ORDER BY label")
            .bind(question_ids)
            .fetch_all(&state.pool)
            .await?;

    for choice in choice_rows {
        question_map
            .entry(choice.question_id)
            .or_default()
            .push(QuestionChoiceResponse {
                label: choice.label,
                text: choice.text,
            });
    }

    Ok(question_map)
}
