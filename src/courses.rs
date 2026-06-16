use crate::{
    models::{CourseListResponse, CourseResponse, CreateCourseRequest},
    AppError, AppResult, AppState,
};
use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct CourseOverviewRow {
    id: Uuid,
    title: String,
    description: Option<String>,
    created_at: DateTime<Utc>,
    video_count: i64,
    question_count: i64,
}

#[utoipa::path(
    get,
    path = "/api/courses",
    tag = "Courses",
    responses(
        (status = 200, description = "Courses with video and question counts", body = CourseListResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_courses(State(state): State<AppState>) -> AppResult<Json<CourseListResponse>> {
    let rows: Vec<CourseOverviewRow> = sqlx::query_as(course_overview_sql())
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(CourseListResponse {
        courses: rows.into_iter().map(CourseResponse::from).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/courses",
    tag = "Courses",
    request_body = CreateCourseRequest,
    responses(
        (status = 200, description = "Course created", body = CourseResponse),
        (status = 400, description = "Bad request"),
        (status = 409, description = "Course already exists"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_course(
    State(state): State<AppState>,
    Json(payload): Json<CreateCourseRequest>,
) -> AppResult<Json<CourseResponse>> {
    let title = payload.title.trim();
    if title.is_empty() {
        return Err(AppError::bad_request("course title cannot be empty"));
    }
    let description = payload
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let insert_result = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO courses (title, description)
        VALUES ($1, $2)
        RETURNING id
        "#,
    )
    .bind(title)
    .bind(description)
    .fetch_one(&state.pool)
    .await;

    let course_id = match insert_result {
        Ok(course_id) => course_id,
        Err(sqlx::Error::Database(error)) if error.constraint() == Some("courses_title_key") => {
            return Err(AppError::conflict(format!(
                "course title {title:?} already exists"
            )));
        }
        Err(error) => return Err(error.into()),
    };

    let row: CourseOverviewRow = sqlx::query_as(
        r#"
        SELECT
            c.id,
            c.title,
            c.description,
            c.created_at,
            COUNT(DISTINCT v.id)::BIGINT AS video_count,
            COUNT(DISTINCT q.id)::BIGINT AS question_count
        FROM courses c
        LEFT JOIN videos v ON v.course_id = c.id
        LEFT JOIN questions q ON q.video_id = v.id
        WHERE c.id = $1
        GROUP BY c.id
        "#,
    )
    .bind(course_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(row.into()))
}

fn course_overview_sql() -> &'static str {
    r#"
    SELECT
        c.id,
        c.title,
        c.description,
        c.created_at,
        COUNT(DISTINCT v.id)::BIGINT AS video_count,
        COUNT(DISTINCT q.id)::BIGINT AS question_count
    FROM courses c
    LEFT JOIN videos v ON v.course_id = c.id
    LEFT JOIN questions q ON q.video_id = v.id
    GROUP BY c.id
    ORDER BY c.created_at DESC
    "#
}

impl From<CourseOverviewRow> for CourseResponse {
    fn from(row: CourseOverviewRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            description: row.description,
            created_at: row.created_at,
            video_count: row.video_count,
            question_count: row.question_count,
        }
    }
}
