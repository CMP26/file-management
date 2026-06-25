use crate::{
    models::{CourseListResponse, CourseResponse, CreateCourseRequest, DeleteCourseResponse},
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
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
    document_count: i64,
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
            COUNT(DISTINCT d.id)::BIGINT AS document_count,
            COUNT(DISTINCT q.id)::BIGINT AS question_count
        FROM courses c
        LEFT JOIN videos v ON v.course_id = c.id
        LEFT JOIN documents d ON d.course_id = c.id
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

#[utoipa::path(
    delete,
    path = "/api/courses/{course_id}",
    tag = "Courses",
    params(
        ("course_id" = Uuid, Path, description = "Course id")
    ),
    responses(
        (status = 200, description = "Course deleted", body = DeleteCourseResponse),
        (status = 404, description = "Course not found"),
        (status = 409, description = "Course still contains videos"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_course(
    State(state): State<AppState>,
    Path(course_id): Path<Uuid>,
) -> AppResult<Json<DeleteCourseResponse>> {
    let mut transaction = state.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM courses WHERE id = $1 FOR UPDATE")
        .bind(course_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::not_found(format!("course {course_id} was not found")))?;

    let (video_count, document_count) = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            (SELECT COUNT(*) FROM videos WHERE course_id = $1),
            (SELECT COUNT(*) FROM documents WHERE course_id = $1)
        "#,
    )
    .bind(course_id)
    .fetch_one(&mut *transaction)
    .await?;
    if video_count > 0 || document_count > 0 {
        return Err(AppError::conflict(format!(
            "course {course_id} contains {video_count} video(s) and {document_count} document(s); delete them first"
        )));
    }

    sqlx::query("DELETE FROM courses WHERE id = $1")
        .bind(course_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    Ok(Json(DeleteCourseResponse {
        course_id,
        deleted: true,
    }))
}

fn course_overview_sql() -> &'static str {
    r#"
    SELECT
        c.id,
        c.title,
        c.description,
        c.created_at,
        COUNT(DISTINCT v.id)::BIGINT AS video_count,
        COUNT(DISTINCT d.id)::BIGINT AS document_count,
        COUNT(DISTINCT q.id)::BIGINT AS question_count
    FROM courses c
    LEFT JOIN videos v ON v.course_id = c.id
    LEFT JOIN documents d ON d.course_id = c.id
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
            document_count: row.document_count,
            question_count: row.question_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::course_overview_sql;

    #[test]
    fn course_overview_counts_videos_documents_and_questions() {
        let sql = course_overview_sql();

        assert!(sql.contains("COUNT(DISTINCT v.id)::BIGINT AS video_count"));
        assert!(sql.contains("COUNT(DISTINCT d.id)::BIGINT AS document_count"));
        assert!(sql.contains("COUNT(DISTINCT q.id)::BIGINT AS question_count"));
        assert!(sql.contains("ORDER BY c.created_at DESC"));
    }
}
