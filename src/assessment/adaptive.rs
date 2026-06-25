use crate::{
    assessment::{
        grader::grade_answer,
        performance::{category_for_optional_score, score_percent},
    },
    models::{
        AdaptiveAnswerResponse, AdaptiveExamStatusResponse, AdaptiveQuestionResponse,
        GradeResponse, QuestionChoiceResponse, StartAdaptiveExamRequest,
        SubmitAdaptiveAnswerRequest,
    },
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

const DEFAULT_MAX_ADAPTIVE_QUESTIONS: i32 = 10;
const MAX_ADAPTIVE_QUESTIONS: i32 = 30;
const IRT_UPDATE_STEP: f64 = 0.8;
const MIN_THETA: f64 = -3.0;
const MAX_THETA: f64 = 3.0;

#[derive(Debug, FromRow)]
struct AdaptiveAttemptRow {
    id: Uuid,
    user_id: Uuid,
    video_id: Option<Uuid>,
    course_id: Uuid,
    ability_theta: f64,
    standard_error: f64,
    status: String,
    max_questions: i32,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct AdaptiveQuestionRow {
    id: Uuid,
    video_id: Uuid,
    topic_id: Option<Uuid>,
    topic_label: Option<String>,
    stem: String,
    question_type: String,
    difficulty: Option<String>,
}

#[derive(Debug, FromRow)]
struct AdaptiveAnswerRow {
    id: Uuid,
    question_id: Uuid,
    video_id: Uuid,
    topic_id: Option<Uuid>,
    topic_label: Option<String>,
    stem: String,
    question_type: String,
    difficulty: Option<String>,
    user_answer: String,
    is_correct: bool,
    score: i16,
    ability_before: f64,
    ability_after: f64,
    item_difficulty: f64,
    answered_at: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/api/videos/{video_id}/adaptive-exams/start",
    tag = "Assessment",
    request_body = StartAdaptiveExamRequest,
    responses(
        (status = 200, description = "Adaptive exam attempt started", body = AdaptiveExamStatusResponse),
        (status = 400, description = "Invalid max question count"),
        (status = 404, description = "Video not found")
    )
)]
pub async fn start_adaptive_exam(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
    Json(payload): Json<StartAdaptiveExamRequest>,
) -> AppResult<Json<AdaptiveExamStatusResponse>> {
    let course_id = video_course_id(&state, video_id).await?;
    let max_questions = payload
        .max_questions
        .unwrap_or(DEFAULT_MAX_ADAPTIVE_QUESTIONS);
    if !(1..=MAX_ADAPTIVE_QUESTIONS).contains(&max_questions) {
        return Err(AppError::bad_request(format!(
            "max_questions must be between 1 and {MAX_ADAPTIVE_QUESTIONS}"
        )));
    }

    let attempt_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO adaptive_exam_attempts (user_id, video_id, course_id, max_questions)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(payload.user_id)
    .bind(video_id)
    .bind(course_id)
    .bind(max_questions)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(adaptive_status_response(&state, attempt_id).await?))
}

#[utoipa::path(
    post,
    path = "/api/courses/{course_id}/adaptive-exams/start",
    tag = "Assessment",
    request_body = StartAdaptiveExamRequest,
    responses(
        (status = 200, description = "Course question-bank adaptive exam started", body = AdaptiveExamStatusResponse),
        (status = 400, description = "Invalid max question count"),
        (status = 404, description = "Course not found")
    )
)]
pub async fn start_course_adaptive_exam(
    State(state): State<AppState>,
    Path(course_id): Path<Uuid>,
    Json(payload): Json<StartAdaptiveExamRequest>,
) -> AppResult<Json<AdaptiveExamStatusResponse>> {
    ensure_course_exists(&state, course_id).await?;
    let max_questions = payload
        .max_questions
        .unwrap_or(DEFAULT_MAX_ADAPTIVE_QUESTIONS);
    if !(1..=MAX_ADAPTIVE_QUESTIONS).contains(&max_questions) {
        return Err(AppError::bad_request(format!(
            "max_questions must be between 1 and {MAX_ADAPTIVE_QUESTIONS}"
        )));
    }

    let attempt_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO adaptive_exam_attempts (user_id, course_id, max_questions)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(payload.user_id)
    .bind(course_id)
    .bind(max_questions)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(adaptive_status_response(&state, attempt_id).await?))
}

#[utoipa::path(
    get,
    path = "/api/adaptive-exams/{attempt_id}",
    tag = "Assessment",
    responses(
        (status = 200, description = "Adaptive exam state with next question", body = AdaptiveExamStatusResponse),
        (status = 404, description = "Adaptive attempt not found")
    )
)]
pub async fn get_adaptive_exam(
    State(state): State<AppState>,
    Path(attempt_id): Path<Uuid>,
) -> AppResult<Json<AdaptiveExamStatusResponse>> {
    Ok(Json(adaptive_status_response(&state, attempt_id).await?))
}

#[utoipa::path(
    post,
    path = "/api/adaptive-exams/{attempt_id}/answer",
    tag = "Assessment",
    request_body = SubmitAdaptiveAnswerRequest,
    responses(
        (status = 200, description = "Answer graded and next adaptive question selected", body = AdaptiveExamStatusResponse),
        (status = 400, description = "Invalid answer or question"),
        (status = 409, description = "Adaptive attempt already completed")
    )
)]
pub async fn submit_adaptive_answer(
    State(state): State<AppState>,
    Path(attempt_id): Path<Uuid>,
    Json(payload): Json<SubmitAdaptiveAnswerRequest>,
) -> AppResult<Json<AdaptiveExamStatusResponse>> {
    let user_answer = payload.user_answer.trim();
    if user_answer.is_empty() {
        return Err(AppError::bad_request("answer cannot be empty"));
    }

    let attempt = load_attempt(&state, attempt_id).await?;
    if attempt.status == "completed" {
        return Err(AppError::conflict("adaptive attempt is already completed"));
    }

    ensure_question_in_attempt_scope(&state, &attempt, payload.question_id).await?;

    let answered_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM adaptive_exam_answers WHERE attempt_id = $1",
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await?;
    if answered_before >= i64::from(attempt.max_questions) {
        complete_attempt(&state, attempt_id).await?;
        return Ok(Json(adaptive_status_response(&state, attempt_id).await?));
    }

    let already_answered: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM adaptive_exam_answers WHERE attempt_id = $1 AND question_id = $2)",
    )
    .bind(attempt_id)
    .bind(payload.question_id)
    .fetch_one(&state.pool)
    .await?;
    if already_answered {
        return Err(AppError::conflict(
            "question has already been answered in this adaptive attempt",
        ));
    }

    let grade = grade_answer(&state, payload.question_id, user_answer)
        .await
        .unwrap_or(GradeResponse {
            is_correct: false,
            score: 0,
        });
    let item_difficulty = question_irt_difficulty(&state, payload.question_id).await?;
    let ability_after = update_ability(
        attempt.ability_theta,
        item_difficulty,
        f64::from(grade.score) / 100.0,
    );
    let answered_after = answered_before + 1;
    let standard_error = standard_error(answered_after);
    let completed = answered_after >= i64::from(attempt.max_questions)
        || remaining_question_count(&state, &attempt).await? <= 1;

    sqlx::query(
        r#"
        INSERT INTO adaptive_exam_answers
            (attempt_id, question_id, user_answer, is_correct, score,
             ability_before, ability_after, item_difficulty)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(attempt_id)
    .bind(payload.question_id)
    .bind(user_answer)
    .bind(grade.is_correct)
    .bind(grade.score)
    .bind(attempt.ability_theta)
    .bind(ability_after)
    .bind(item_difficulty)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        r#"
        UPDATE adaptive_exam_attempts
        SET ability_theta = $1, standard_error = $2,
            status = CASE WHEN $3 THEN 'completed' ELSE status END,
            completed_at = CASE WHEN $3 THEN now() ELSE completed_at END
        WHERE id = $4
        "#,
    )
    .bind(ability_after)
    .bind(standard_error)
    .bind(completed)
    .bind(attempt_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(adaptive_status_response(&state, attempt_id).await?))
}

async fn adaptive_status_response(
    state: &AppState,
    attempt_id: Uuid,
) -> AppResult<AdaptiveExamStatusResponse> {
    let attempt = load_attempt(state, attempt_id).await?;
    let answers = load_adaptive_answers(state, attempt_id).await?;
    let answered_count = answers.len() as i64;
    let total_score = answers
        .iter()
        .map(|answer| i32::from(answer.score))
        .sum::<i32>();
    let performance_score = score_percent(total_score, answered_count);
    let next_question =
        if attempt.status == "active" && answered_count < i64::from(attempt.max_questions) {
            select_next_question(state, &attempt).await?
        } else {
            None
        };

    Ok(AdaptiveExamStatusResponse {
        attempt_id: attempt.id,
        user_id: attempt.user_id,
        video_id: attempt.video_id,
        course_id: attempt.course_id,
        status: attempt.status,
        ability_theta: attempt.ability_theta,
        standard_error: attempt.standard_error,
        score_percent: performance_score,
        performance_category: category_for_optional_score(performance_score),
        max_questions: attempt.max_questions,
        answered_count,
        started_at: attempt.started_at,
        completed_at: attempt.completed_at,
        next_question,
        answers,
    })
}

async fn select_next_question(
    state: &AppState,
    attempt: &AdaptiveAttemptRow,
) -> AppResult<Option<AdaptiveQuestionResponse>> {
    let rows = sqlx::query_as::<_, AdaptiveQuestionRow>(
        r#"
        SELECT q.id, q.video_id, q.topic_id, t.label AS topic_label,
               q.stem, q.question_type, q.difficulty
        FROM questions q
        LEFT JOIN topics t ON t.id = q.topic_id
        JOIN videos v ON v.id = q.video_id
        WHERE ($1::UUID IS NULL OR q.video_id = $1)
          AND v.course_id = $2
          AND NOT EXISTS (
              SELECT 1
              FROM adaptive_exam_answers aa
              WHERE aa.attempt_id = $3 AND aa.question_id = q.id
          )
        "#,
    )
    .bind(attempt.video_id)
    .bind(attempt.course_id)
    .bind(attempt.id)
    .fetch_all(&state.pool)
    .await?;

    let Some(row) = rows.into_iter().min_by(|left, right| {
        let left_distance =
            (difficulty_to_irt(left.difficulty.as_deref()) - attempt.ability_theta).abs();
        let right_distance =
            (difficulty_to_irt(right.difficulty.as_deref()) - attempt.ability_theta).abs();
        left_distance
            .partial_cmp(&right_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.stem.cmp(&right.stem))
    }) else {
        return Ok(None);
    };

    let choice_map = load_choice_map(state, &[row.id]).await?;
    Ok(Some(question_response(row, &choice_map)))
}

async fn load_adaptive_answers(
    state: &AppState,
    attempt_id: Uuid,
) -> AppResult<Vec<AdaptiveAnswerResponse>> {
    let rows = sqlx::query_as::<_, AdaptiveAnswerRow>(
        r#"
        SELECT aa.id, aa.question_id, q.video_id, q.topic_id, t.label AS topic_label,
               q.stem, q.question_type, q.difficulty,
               aa.user_answer, aa.is_correct, aa.score,
               aa.ability_before, aa.ability_after, aa.item_difficulty, aa.answered_at
        FROM adaptive_exam_answers aa
        JOIN questions q ON q.id = aa.question_id
        LEFT JOIN topics t ON t.id = q.topic_id
        WHERE aa.attempt_id = $1
        ORDER BY aa.answered_at ASC
        "#,
    )
    .bind(attempt_id)
    .fetch_all(&state.pool)
    .await?;

    let question_ids = rows.iter().map(|row| row.question_id).collect::<Vec<_>>();
    let choice_map = load_choice_map(state, &question_ids).await?;

    Ok(rows
        .into_iter()
        .map(|row| AdaptiveAnswerResponse {
            answer_id: row.id,
            question_id: row.question_id,
            question: Some(question_response(
                AdaptiveQuestionRow {
                    id: row.question_id,
                    video_id: row.video_id,
                    topic_id: row.topic_id,
                    topic_label: row.topic_label,
                    stem: row.stem,
                    question_type: row.question_type,
                    difficulty: row.difficulty,
                },
                &choice_map,
            )),
            user_answer: row.user_answer,
            is_correct: row.is_correct,
            score: row.score,
            ability_before: row.ability_before,
            ability_after: row.ability_after,
            item_difficulty: row.item_difficulty,
            answered_at: row.answered_at,
        })
        .collect())
}

async fn question_irt_difficulty(state: &AppState, question_id: Uuid) -> AppResult<f64> {
    let difficulty: Option<String> =
        sqlx::query_scalar("SELECT difficulty FROM questions WHERE id = $1")
            .bind(question_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(difficulty_to_irt(difficulty.as_deref()))
}

async fn remaining_question_count(
    state: &AppState,
    attempt: &AdaptiveAttemptRow,
) -> AppResult<i64> {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::BIGINT
        FROM questions q
        JOIN videos v ON v.id = q.video_id
        WHERE ($1::UUID IS NULL OR q.video_id = $1)
          AND v.course_id = $2
          AND NOT EXISTS (
              SELECT 1
              FROM adaptive_exam_answers aa
              WHERE aa.attempt_id = $3 AND aa.question_id = q.id
          )
        "#,
    )
    .bind(attempt.video_id)
    .bind(attempt.course_id)
    .bind(attempt.id)
    .fetch_one(&state.pool)
    .await
    .map_err(Into::into)
}

async fn load_attempt(state: &AppState, attempt_id: Uuid) -> AppResult<AdaptiveAttemptRow> {
    sqlx::query_as("SELECT * FROM adaptive_exam_attempts WHERE id = $1")
        .bind(attempt_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found(format!("adaptive attempt {attempt_id} was not found")))
}

async fn complete_attempt(state: &AppState, attempt_id: Uuid) -> AppResult<()> {
    sqlx::query(
        r#"
        UPDATE adaptive_exam_attempts
        SET status = 'completed', completed_at = COALESCE(completed_at, now())
        WHERE id = $1
        "#,
    )
    .bind(attempt_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn video_course_id(state: &AppState, video_id: Uuid) -> AppResult<Uuid> {
    sqlx::query_scalar("SELECT course_id FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found(format!("video {video_id} was not found")))
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

async fn ensure_question_in_attempt_scope(
    state: &AppState,
    attempt: &AdaptiveAttemptRow,
    question_id: Uuid,
) -> AppResult<()> {
    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        r#"
        SELECT q.video_id, v.course_id
        FROM questions q
        JOIN videos v ON v.id = q.video_id
        WHERE q.id = $1
        "#,
    )
    .bind(question_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((question_video_id, question_course_id)) = row else {
        return Err(AppError::not_found(format!(
            "question {question_id} was not found"
        )));
    };
    if question_course_id != attempt.course_id {
        return Err(AppError::bad_request(
            "adaptive question must belong to the attempt course",
        ));
    }
    if let Some(video_id) = attempt.video_id {
        if question_video_id != video_id {
            return Err(AppError::bad_request(
                "adaptive question must belong to the attempt video",
            ));
        }
    }
    Ok(())
}

async fn load_choice_map(
    state: &AppState,
    question_ids: &[Uuid],
) -> AppResult<HashMap<Uuid, Vec<QuestionChoiceResponse>>> {
    let mut question_map = HashMap::new();
    if question_ids.is_empty() {
        return Ok(question_map);
    }

    let rows = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT question_id, label, text FROM choices WHERE question_id = ANY($1) ORDER BY label",
    )
    .bind(question_ids)
    .fetch_all(&state.pool)
    .await?;
    for (question_id, label, text) in rows {
        question_map
            .entry(question_id)
            .or_insert_with(Vec::new)
            .push(QuestionChoiceResponse { label, text });
    }
    Ok(question_map)
}

fn question_response(
    row: AdaptiveQuestionRow,
    choice_map: &HashMap<Uuid, Vec<QuestionChoiceResponse>>,
) -> AdaptiveQuestionResponse {
    let irt_difficulty = difficulty_to_irt(row.difficulty.as_deref());
    AdaptiveQuestionResponse {
        id: row.id,
        video_id: row.video_id,
        topic_id: row.topic_id,
        topic_label: row.topic_label,
        stem: row.stem,
        question_type: row.question_type,
        difficulty: row.difficulty,
        irt_difficulty,
        choices: choice_map.get(&row.id).cloned().unwrap_or_default(),
    }
}

fn difficulty_to_irt(difficulty: Option<&str>) -> f64 {
    match difficulty
        .unwrap_or("medium")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "easy" => -1.0,
        "hard" => 1.0,
        _ => 0.0,
    }
}

fn one_pl_probability(theta: f64, difficulty: f64) -> f64 {
    1.0 / (1.0 + (-(theta - difficulty)).exp())
}

fn update_ability(theta: f64, difficulty: f64, observed_score: f64) -> f64 {
    let expected = one_pl_probability(theta, difficulty);
    (theta + IRT_UPDATE_STEP * (observed_score.clamp(0.0, 1.0) - expected))
        .clamp(MIN_THETA, MAX_THETA)
}

fn standard_error(answered_count: i64) -> f64 {
    1.0 / (answered_count.max(1) as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::{difficulty_to_irt, one_pl_probability, standard_error, update_ability};

    #[test]
    fn maps_text_difficulty_to_irt_scale() {
        assert_eq!(difficulty_to_irt(Some("easy")), -1.0);
        assert_eq!(difficulty_to_irt(Some("medium")), 0.0);
        assert_eq!(difficulty_to_irt(Some("hard")), 1.0);
        assert_eq!(difficulty_to_irt(Some("unknown")), 0.0);
    }

    #[test]
    fn one_pl_probability_increases_when_ability_exceeds_difficulty() {
        assert!(one_pl_probability(1.0, 0.0) > one_pl_probability(0.0, 1.0));
    }

    #[test]
    fn ability_moves_up_for_correct_and_down_for_incorrect() {
        assert!(update_ability(0.0, 0.0, 1.0) > 0.0);
        assert!(update_ability(0.0, 0.0, 0.0) < 0.0);
        assert_eq!(update_ability(3.0, -1.0, 1.0), 3.0);
    }

    #[test]
    fn standard_error_shrinks_with_more_answers() {
        assert!(standard_error(4) < standard_error(1));
    }
}
