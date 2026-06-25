use crate::{
    assessment::{
        grader::grade_answer,
        justifier::response_for_answer,
        performance::{category_for_optional_score, score_percent},
    },
    models::{
        AttemptAnswerRecord, AttemptAnswerStatusItem, AttemptStatusResponse, ChoiceRecord,
        CourseRandomQuestionResponse, CourseRandomQuestionsResponse, DeleteExamAttemptResponse,
        ExamAttemptRecord, GradeResponse, JustificationResponse, JustificationStatusResponse,
        QuestionChoiceResponse, QuestionRecord, QuestionsByVideoResponse, SourceVideoResponse,
        StartExamRequest, StartExamResponse, SubmitAttemptRequest, SubmitAttemptResponse,
        TopicQuestionGroupResponse, UserExamAttemptListResponse, UserExamAttemptResponse,
    },
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use sqlx::FromRow;
use std::collections::HashMap;
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
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

#[derive(Debug, Default, serde::Deserialize)]
pub struct UserExamFilters {
    pub video_id: Option<Uuid>,
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

#[derive(Debug, FromRow)]
struct UserExamAttemptRow {
    attempt_id: Uuid,
    user_id: Uuid,
    video_id: Uuid,
    video_title: String,
    course_id: Uuid,
    course_title: String,
    started_at: chrono::DateTime<chrono::Utc>,
    submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    total_score: i32,
    pending_count: i64,
    answer_count: i64,
    graded_count: i64,
}

#[derive(Debug, FromRow)]
struct UserAdaptiveAttemptRow {
    attempt_id: Uuid,
    user_id: Uuid,
    video_id: Option<Uuid>,
    video_title: Option<String>,
    course_id: Uuid,
    course_title: String,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    status: String,
    total_score: i32,
    answer_count: i64,
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
    get,
    path = "/api/users/{user_id}/exams",
    tag = "Assessment",
    params(
        ("user_id" = Uuid, Path, description = "User id"),
        ("video_id" = Option<Uuid>, Query, description = "Optionally filter attempts by lesson/video id")
    ),
    responses(
        (status = 200, description = "Assessment attempts for a user", body = UserExamAttemptListResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_user_exam_attempts(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(filters): Query<UserExamFilters>,
) -> AppResult<Json<UserExamAttemptListResponse>> {
    let rows: Vec<UserExamAttemptRow> = sqlx::query_as(
        r#"
        SELECT
            ea.id AS attempt_id,
            ea.user_id,
            ea.video_id,
            v.title AS video_title,
            c.id AS course_id,
            c.title AS course_title,
            ea.started_at,
            ea.submitted_at,
            COALESCE(SUM(aa.score), 0)::INTEGER AS total_score,
            COUNT(aa.id) FILTER (WHERE aa.graded_at IS NULL)::BIGINT AS pending_count,
            COUNT(aa.id)::BIGINT AS answer_count,
            COUNT(aa.score)::BIGINT AS graded_count
        FROM exam_attempts ea
        JOIN videos v ON v.id = ea.video_id
        JOIN courses c ON c.id = v.course_id
        LEFT JOIN attempt_answers aa ON aa.attempt_id = ea.id
        WHERE ea.user_id = $1
          AND ($2::UUID IS NULL OR ea.video_id = $2)
        GROUP BY ea.id, ea.user_id, ea.video_id, v.title, c.id, c.title, ea.started_at, ea.submitted_at
        ORDER BY ea.started_at DESC
        "#,
    )
    .bind(user_id)
    .bind(filters.video_id)
    .fetch_all(&state.pool)
    .await?;

    let adaptive_rows: Vec<UserAdaptiveAttemptRow> = sqlx::query_as(
        r#"
        SELECT
            a.id AS attempt_id,
            a.user_id,
            a.video_id,
            v.title AS video_title,
            c.id AS course_id,
            c.title AS course_title,
            a.started_at,
            a.completed_at,
            a.status,
            COALESCE(SUM(aa.score), 0)::INTEGER AS total_score,
            COUNT(aa.id)::BIGINT AS answer_count
        FROM adaptive_exam_attempts a
        JOIN courses c ON c.id = a.course_id
        LEFT JOIN videos v ON v.id = a.video_id
        LEFT JOIN adaptive_exam_answers aa ON aa.attempt_id = a.id
        WHERE a.user_id = $1
          AND ($2::UUID IS NULL OR a.video_id = $2)
        GROUP BY a.id, a.user_id, a.video_id, v.title, c.id, c.title, a.started_at, a.completed_at, a.status
        ORDER BY a.started_at DESC
        "#,
    )
    .bind(user_id)
    .bind(filters.video_id)
    .fetch_all(&state.pool)
    .await?;

    let mut attempts = rows
        .into_iter()
        .map(|row| {
            let (status, is_waiting) =
                attempt_status_fields(row.submitted_at.is_some(), row.pending_count);
            let score_percent = score_percent(row.total_score, row.graded_count);
            UserExamAttemptResponse {
                attempt_id: row.attempt_id,
                assessment_type: "batch".to_string(),
                user_id: row.user_id,
                video_id: Some(row.video_id),
                video_title: row.video_title,
                course_id: row.course_id,
                course_title: row.course_title,
                started_at: row.started_at,
                submitted_at: row.submitted_at,
                status: status.to_string(),
                is_waiting,
                total_score: row.total_score,
                score_percent,
                performance_category: category_for_optional_score(score_percent),
                pending_count: row.pending_count,
                answer_count: row.answer_count,
            }
        })
        .collect::<Vec<_>>();

    attempts.extend(adaptive_rows.into_iter().map(|row| {
        let score_percent = score_percent(row.total_score, row.answer_count);
        UserExamAttemptResponse {
            attempt_id: row.attempt_id,
            assessment_type: "adaptive".to_string(),
            user_id: row.user_id,
            video_id: row.video_id,
            video_title: row
                .video_title
                .unwrap_or_else(|| "Course adaptive assessment".to_string()),
            course_id: row.course_id,
            course_title: row.course_title,
            started_at: row.started_at,
            submitted_at: row.completed_at,
            status: row.status.clone(),
            is_waiting: row.status == "active",
            total_score: row.total_score,
            score_percent,
            performance_category: category_for_optional_score(score_percent),
            pending_count: 0,
            answer_count: row.answer_count,
        }
    }));

    attempts.sort_by(|left, right| right.started_at.cmp(&left.started_at));

    let completed_scores = attempts
        .iter()
        .filter(|attempt| !attempt.is_waiting)
        .filter_map(|attempt| attempt.score_percent)
        .collect::<Vec<_>>();
    let overall_score_percent = if completed_scores.is_empty() {
        None
    } else {
        Some(completed_scores.iter().sum::<f64>() / completed_scores.len() as f64)
    };
    let overall_category = category_for_optional_score(overall_score_percent);

    Ok(Json(UserExamAttemptListResponse {
        user_id,
        overall_score_percent,
        overall_category,
        completed_assessment_count: completed_scores.len() as i64,
        attempts,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/users/{user_id}/exams/{attempt_id}",
    tag = "Assessment",
    params(
        ("user_id" = Uuid, Path, description = "User id"),
        ("attempt_id" = Uuid, Path, description = "Exam attempt id")
    ),
    responses(
        (status = 200, description = "Deleted exam attempt", body = DeleteExamAttemptResponse),
        (status = 404, description = "Attempt not found for this user"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_user_exam_attempt(
    State(state): State<AppState>,
    Path((user_id, attempt_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<DeleteExamAttemptResponse>> {
    let mut deleted = sqlx::query("DELETE FROM exam_attempts WHERE id = $1 AND user_id = $2")
        .bind(attempt_id)
        .bind(user_id)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if deleted == 0 {
        deleted = sqlx::query("DELETE FROM adaptive_exam_attempts WHERE id = $1 AND user_id = $2")
            .bind(attempt_id)
            .bind(user_id)
            .execute(&state.pool)
            .await?
            .rows_affected();
    }

    if deleted == 0 {
        return Err(AppError::not_found(format!(
            "attempt {attempt_id} was not found for user {user_id}"
        )));
    }

    let _ = state.exam_events.send(attempt_id);

    Ok(Json(DeleteExamAttemptResponse {
        attempt_id,
        deleted: true,
    }))
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
        (status = 400, description = "Question does not belong to the attempt course"),
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

    let existing_answers: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM attempt_answers WHERE attempt_id = $1")
            .bind(attempt_id)
            .fetch_one(&state.pool)
            .await?;
    if existing_answers > 0 {
        return Err(AppError::conflict("attempt has already been submitted"));
    }

    let attempt_course_id: Uuid = sqlx::query_scalar("SELECT course_id FROM videos WHERE id = $1")
        .bind(attempt.video_id)
        .fetch_one(&state.pool)
        .await?;

    let mut submitted_count = 0i64;
    for answer_input in payload.answers {
        let (question_video_id, question_course_id): (Uuid, Uuid) = sqlx::query_as(
            r#"
            SELECT q.video_id, v.course_id
            FROM questions q
            JOIN videos v ON v.id = q.video_id
            WHERE q.id = $1
            "#,
        )
        .bind(answer_input.question_id)
        .fetch_one(&state.pool)
        .await?;
        validate_question_course(
            answer_input.question_id,
            question_video_id,
            question_course_id,
            attempt_course_id,
        )?;

        sqlx::query(
            r#"
            INSERT INTO attempt_answers (attempt_id, question_id, user_answer)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(attempt_id)
        .bind(answer_input.question_id)
        .bind(&answer_input.user_answer)
        .execute(&state.pool)
        .await?;
        submitted_count += 1;
    }

    sqlx::query("UPDATE exam_attempts SET submitted_at = now() WHERE id = $1")
        .bind(attempt.id)
        .execute(&state.pool)
        .await?;

    let _ = state.exam_events.send(attempt_id);
    let worker_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = grade_attempt_answers(worker_state, attempt_id).await {
            tracing::error!(attempt_id = %attempt_id, error = %error, "attempt grading worker failed");
        }
    });

    Ok(Json(SubmitAttemptResponse {
        attempt_id,
        status: "grading".to_string(),
        is_waiting: true,
        pending_count: submitted_count,
        total_score: 0,
        score_percent: None,
        performance_category: None,
        breakdown: Vec::new(),
    }))
}

async fn grade_attempt_answers(state: AppState, attempt_id: Uuid) -> AppResult<()> {
    tracing::info!(attempt_id = %attempt_id, "attempt grading worker started");
    let answers: Vec<AttemptAnswerRecord> = sqlx::query_as(
        "SELECT * FROM attempt_answers WHERE attempt_id = $1 AND graded_at IS NULL ORDER BY id",
    )
    .bind(attempt_id)
    .fetch_all(&state.pool)
    .await?;

    for answer in answers {
        tracing::info!(attempt_id = %attempt_id, answer_id = %answer.id, question_id = %answer.question_id, "grading attempt answer");
        let grade = match grade_answer(&state, answer.question_id, &answer.user_answer).await {
            Ok(grade) => grade,
            Err(error) => {
                tracing::error!(
                    attempt_id = %attempt_id,
                    answer_id = %answer.id,
                    question_id = %answer.question_id,
                    error = %error,
                    "answer grading failed; saving fallback grade"
                );
                GradeResponse {
                    is_correct: false,
                    score: 0,
                }
            }
        };
        sqlx::query(
            r#"
            UPDATE attempt_answers
            SET is_correct = $1, score = $2, graded_at = now()
            WHERE id = $3
            "#,
        )
        .bind(grade.is_correct)
        .bind(grade.score)
        .bind(answer.id)
        .execute(&state.pool)
        .await?;
        tracing::info!(attempt_id = %attempt_id, answer_id = %answer.id, score = grade.score, is_correct = grade.is_correct, "attempt answer graded");
        let _ = state.exam_events.send(attempt_id);
    }

    let _ = state.exam_events.send(attempt_id);
    tracing::info!(attempt_id = %attempt_id, "attempt grading worker completed");
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/exams/{attempt_id}",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id")
    ),
    responses(
        (status = 200, description = "Exam attempt grading status", body = AttemptStatusResponse),
        (status = 404, description = "Attempt not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_attempt_status(
    State(state): State<AppState>,
    Path(attempt_id): Path<Uuid>,
) -> AppResult<Json<AttemptStatusResponse>> {
    Ok(Json(attempt_status_response(&state, attempt_id).await?))
}

#[utoipa::path(
    get,
    path = "/api/exams/{attempt_id}/events",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id")
    ),
    responses(
        (status = 200, description = "Server-sent exam grading updates"),
        (status = 404, description = "Attempt not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn stream_attempt_events(
    State(state): State<AppState>,
    Path(attempt_id): Path<Uuid>,
) -> AppResult<Sse<ReceiverStream<Result<Event, Infallible>>>> {
    let mut exam_events = state.exam_events.subscribe();
    let initial = attempt_status_response(&state, attempt_id).await?;
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let worker_state = state.clone();

    tokio::spawn(async move {
        let _ = send_attempt_event(&tx, &initial).await;
        if !initial.is_waiting {
            return;
        }

        loop {
            match exam_events.recv().await {
                Ok(changed_attempt_id) if changed_attempt_id == attempt_id => {
                    match attempt_status_response(&worker_state, attempt_id).await {
                        Ok(snapshot) => {
                            let is_waiting = snapshot.is_waiting;
                            if send_attempt_event(&tx, &snapshot).await.is_err() {
                                return;
                            }
                            if !is_waiting {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = tx
                                .send(Ok(Event::default().event("error").data(error.to_string())))
                                .await;
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(error.to_string())))
                        .await;
                    return;
                }
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
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

#[utoipa::path(
    post,
    path = "/api/exams/{attempt_id}/answers/{answer_id}/justification/start",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id"),
        ("answer_id" = Uuid, Path, description = "Answer id")
    ),
    responses(
        (status = 200, description = "Justification generation status", body = JustificationStatusResponse),
        (status = 404, description = "Answer not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn start_justification(
    State(state): State<AppState>,
    Path((attempt_id, answer_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<JustificationStatusResponse>> {
    ensure_attempt_answer(&state, attempt_id, answer_id).await?;
    let current = justification_status_response(&state, answer_id).await?;
    if current.justification.is_some() {
        return Ok(Json(current));
    }

    let worker_state = state.clone();
    tokio::spawn(async move {
        tracing::info!(attempt_id = %attempt_id, answer_id = %answer_id, "justification worker started");
        if let Err(error) = response_for_answer(&worker_state, attempt_id, answer_id).await {
            tracing::error!(attempt_id = %attempt_id, answer_id = %answer_id, error = %error, "justification worker failed");
            let fallback = format!(
                "I could not generate a justification for this answer. Please try again. Error: {error}"
            );
            let _ = sqlx::query(
                r#"
                INSERT INTO answer_justifications (attempt_answer_id, justification)
                VALUES ($1, $2)
                ON CONFLICT (attempt_answer_id) DO NOTHING
                "#,
            )
            .bind(answer_id)
            .bind(fallback)
            .execute(&worker_state.pool)
            .await;
        } else {
            tracing::info!(attempt_id = %attempt_id, answer_id = %answer_id, "justification worker completed");
        }
        let _ = worker_state.justification_events.send(answer_id);
    });

    Ok(Json(JustificationStatusResponse {
        answer_id,
        status: "generating".to_string(),
        is_waiting: true,
        justification: None,
    }))
}

#[utoipa::path(
    get,
    path = "/api/exams/{attempt_id}/answers/{answer_id}/justification/status",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id"),
        ("answer_id" = Uuid, Path, description = "Answer id")
    ),
    responses(
        (status = 200, description = "Justification source-of-truth status", body = JustificationStatusResponse),
        (status = 404, description = "Answer not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_justification_status(
    State(state): State<AppState>,
    Path((attempt_id, answer_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<JustificationStatusResponse>> {
    ensure_attempt_answer(&state, attempt_id, answer_id).await?;
    Ok(Json(
        justification_status_response(&state, answer_id).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/exams/{attempt_id}/answers/{answer_id}/justification/events",
    tag = "Assessment",
    params(
        ("attempt_id" = Uuid, Path, description = "Exam attempt id"),
        ("answer_id" = Uuid, Path, description = "Answer id")
    ),
    responses(
        (status = 200, description = "Server-sent justification updates"),
        (status = 404, description = "Answer not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn stream_justification_events(
    State(state): State<AppState>,
    Path((attempt_id, answer_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Sse<ReceiverStream<Result<Event, Infallible>>>> {
    ensure_attempt_answer(&state, attempt_id, answer_id).await?;
    let mut justification_events = state.justification_events.subscribe();
    let initial = justification_status_response(&state, answer_id).await?;
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let worker_state = state.clone();

    tokio::spawn(async move {
        let _ = send_justification_event(&tx, &initial).await;
        if !initial.is_waiting {
            return;
        }

        loop {
            match justification_events.recv().await {
                Ok(changed_answer_id) if changed_answer_id == answer_id => {
                    match justification_status_response(&worker_state, answer_id).await {
                        Ok(snapshot) => {
                            let is_waiting = snapshot.is_waiting;
                            if send_justification_event(&tx, &snapshot).await.is_err() {
                                return;
                            }
                            if !is_waiting {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = tx
                                .send(Ok(Event::default().event("error").data(error.to_string())))
                                .await;
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(error.to_string())))
                        .await;
                    return;
                }
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

async fn attempt_status_response(
    state: &AppState,
    attempt_id: Uuid,
) -> AppResult<AttemptStatusResponse> {
    let attempt: ExamAttemptRecord = sqlx::query_as("SELECT * FROM exam_attempts WHERE id = $1")
        .bind(attempt_id)
        .fetch_one(&state.pool)
        .await?;

    let answers: Vec<AttemptAnswerRecord> =
        sqlx::query_as("SELECT * FROM attempt_answers WHERE attempt_id = $1 ORDER BY id")
            .bind(attempt_id)
            .fetch_all(&state.pool)
            .await?;

    let pending_count = answers
        .iter()
        .filter(|answer| answer.graded_at.is_none())
        .count() as i64;
    let total_score = answers
        .iter()
        .filter_map(|answer| answer.score)
        .map(i32::from)
        .sum();
    let graded_count = answers
        .iter()
        .filter(|answer| answer.score.is_some())
        .count() as i64;
    let score_percent = score_percent(total_score, graded_count);
    let (status, is_waiting) = attempt_status_fields(attempt.submitted_at.is_some(), pending_count);

    Ok(AttemptStatusResponse {
        attempt_id,
        user_id: attempt.user_id,
        video_id: attempt.video_id,
        started_at: attempt.started_at,
        submitted_at: attempt.submitted_at,
        status: status.to_string(),
        is_waiting,
        total_score,
        score_percent,
        performance_category: category_for_optional_score(score_percent),
        pending_count,
        answers: answers
            .into_iter()
            .map(|answer| AttemptAnswerStatusItem {
                answer_id: answer.id,
                question_id: answer.question_id,
                user_answer: answer.user_answer,
                is_correct: answer.is_correct,
                score: answer.score,
                graded_at: answer.graded_at,
            })
            .collect(),
    })
}

async fn send_attempt_event(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    snapshot: &AttemptStatusResponse,
) -> Result<(), mpsc::error::SendError<Result<Event, Infallible>>> {
    let data = serde_json::to_string(snapshot).unwrap_or_else(|error| {
        format!(
            "{{\"error\":\"failed to serialize attempt snapshot\",\"message\":\"{}\"}}",
            error
        )
    });
    tx.send(Ok(Event::default().event("exam").data(data))).await
}

async fn ensure_attempt_answer(
    state: &AppState,
    attempt_id: Uuid,
    answer_id: Uuid,
) -> AppResult<()> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM attempt_answers WHERE id = $1 AND attempt_id = $2)",
    )
    .bind(answer_id)
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await?;

    if exists {
        Ok(())
    } else {
        Err(AppError::not_found(format!(
            "answer {answer_id} was not found for attempt {attempt_id}"
        )))
    }
}

async fn justification_status_response(
    state: &AppState,
    answer_id: Uuid,
) -> AppResult<JustificationStatusResponse> {
    let justification = sqlx::query_scalar::<_, String>(
        "SELECT justification FROM answer_justifications WHERE attempt_answer_id = $1",
    )
    .bind(answer_id)
    .fetch_optional(&state.pool)
    .await?;

    let is_waiting = justification.is_none();
    Ok(JustificationStatusResponse {
        answer_id,
        status: if is_waiting {
            "generating".to_string()
        } else {
            "ready".to_string()
        },
        is_waiting,
        justification,
    })
}

async fn send_justification_event(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    snapshot: &JustificationStatusResponse,
) -> Result<(), mpsc::error::SendError<Result<Event, Infallible>>> {
    let data = serde_json::to_string(snapshot).unwrap_or_else(|error| {
        format!(
            "{{\"error\":\"failed to serialize justification snapshot\",\"message\":\"{}\"}}",
            error
        )
    });
    tx.send(Ok(Event::default().event("justification").data(data)))
        .await
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

fn attempt_status_fields(is_submitted: bool, pending_count: i64) -> (&'static str, bool) {
    if !is_submitted {
        ("started", false)
    } else if pending_count > 0 {
        ("grading", true)
    } else {
        ("graded", false)
    }
}

fn validate_question_course(
    question_id: Uuid,
    question_video_id: Uuid,
    question_course_id: Uuid,
    attempt_course_id: Uuid,
) -> AppResult<()> {
    if question_course_id == attempt_course_id {
        Ok(())
    } else {
        Err(AppError::bad_request(format!(
            "question {question_id} from video {question_video_id} does not belong to attempt course {attempt_course_id}"
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

#[cfg(test)]
mod tests {
    use super::{attempt_status_fields, validate_question_course};
    use crate::AppError;
    use uuid::Uuid;

    #[test]
    fn attempt_status_started_is_not_waiting() {
        assert_eq!(attempt_status_fields(false, 0), ("started", false));
        assert_eq!(attempt_status_fields(false, 3), ("started", false));
    }

    #[test]
    fn attempt_status_submitted_with_pending_grades_is_waiting() {
        assert_eq!(attempt_status_fields(true, 2), ("grading", true));
    }

    #[test]
    fn attempt_status_submitted_without_pending_grades_is_graded() {
        assert_eq!(attempt_status_fields(true, 0), ("graded", false));
    }

    #[test]
    fn question_bank_attempt_allows_questions_from_same_course() {
        let question_id = Uuid::new_v4();
        let question_video_id = Uuid::new_v4();
        let course_id = Uuid::new_v4();

        let result = validate_question_course(question_id, question_video_id, course_id, course_id);

        assert!(result.is_ok());
    }

    #[test]
    fn question_bank_attempt_rejects_questions_from_other_courses() {
        let result = validate_question_course(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
