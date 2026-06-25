use crate::{
    models::{
        DeleteDocumentResponse, DocumentListResponse, DocumentResponse, DocumentUploadResponse,
    },
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderValue},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::FromRow;
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

const MAX_PDF_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, FromRow)]
struct DocumentRow {
    id: Uuid,
    course_id: Uuid,
    course_title: String,
    title: String,
    file_name: String,
    content_type: String,
    status: String,
    error_msg: Option<String>,
    page_count: Option<i32>,
    chunk_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DocumentFilters {
    pub course_id: Option<Uuid>,
}

#[utoipa::path(
    post,
    path = "/api/documents/upload",
    tag = "Documents",
    responses(
        (status = 200, description = "PDF uploaded and processing queued", body = DocumentUploadResponse),
        (status = 400, description = "Invalid PDF upload"),
        (status = 404, description = "Course not found")
    )
)]
pub async fn upload_document(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<DocumentUploadResponse>> {
    let mut title = None;
    let mut course_id = None;
    let mut file_name = None;
    let mut content_type = None;
    let mut bytes = None;

    while let Some(field) = multipart.next_field().await? {
        match field.name().unwrap_or_default() {
            "title" => title = Some(field.text().await?),
            "course_id" => {
                course_id = Some(
                    Uuid::parse_str(field.text().await?.trim())
                        .map_err(|_| AppError::bad_request("course_id must be a UUID"))?,
                );
            }
            "file" => {
                file_name = field.file_name().map(str::to_string);
                content_type = field.content_type().map(str::to_string);
                let value = field.bytes().await?;
                if value.len() > MAX_PDF_BYTES {
                    return Err(AppError::bad_request("PDF cannot exceed 100 MiB"));
                }
                bytes = Some(value.to_vec());
            }
            _ => {}
        }
    }

    let course_id = course_id.ok_or_else(|| AppError::bad_request("missing course_id field"))?;
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM courses WHERE id = $1)")
            .bind(course_id)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(AppError::not_found(format!(
            "course {course_id} was not found"
        )));
    }

    let bytes = bytes.ok_or_else(|| AppError::bad_request("missing file field"))?;
    let file_name = file_name.unwrap_or_else(|| "document.pdf".to_string());
    let is_pdf = bytes.starts_with(b"%PDF-")
        && (content_type.as_deref() == Some("application/pdf")
            || file_name.to_ascii_lowercase().ends_with(".pdf"));
    if !is_pdf {
        return Err(AppError::bad_request("only PDF documents are supported"));
    }
    let title = title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| file_name.trim_end_matches(".pdf"));

    let document_id = Uuid::new_v4();
    let rustfs_key = format!("documents/{document_id}/original.pdf");
    state
        .storage
        .upload(&rustfs_key, bytes, "application/pdf")
        .await?;
    sqlx::query(
        r#"
        INSERT INTO documents
            (id, course_id, title, file_name, rustfs_key, content_type, status)
        VALUES ($1, $2, $3, $4, $5, 'application/pdf', 'pending')
        "#,
    )
    .bind(document_id)
    .bind(course_id)
    .bind(title)
    .bind(&file_name)
    .bind(&rustfs_key)
    .execute(&state.pool)
    .await?;

    let worker_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) =
            crate::document_processing::process_document(worker_state, document_id).await
        {
            tracing::error!(document_id = %document_id, error = %error, "document processing failed");
        }
    });
    let _ = state.document_events.send(document_id);

    Ok(Json(DocumentUploadResponse {
        document_id,
        course_id,
        status: "pending".to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/documents",
    tag = "Documents",
    params(("course_id" = Option<Uuid>, Query, description = "Optional course filter")),
    responses((status = 200, body = DocumentListResponse))
)]
pub async fn list_documents(
    State(state): State<AppState>,
    Query(filters): Query<DocumentFilters>,
) -> AppResult<Json<DocumentListResponse>> {
    let rows: Vec<DocumentRow> = if let Some(course_id) = filters.course_id {
        sqlx::query_as(&format!(
            "{} WHERE d.course_id = $1 {}",
            document_select(),
            document_order()
        ))
        .bind(course_id)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as(&format!("{} {}", document_select(), document_order()))
            .fetch_all(&state.pool)
            .await?
    };
    Ok(Json(DocumentListResponse {
        documents: rows.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/documents/{document_id}",
    tag = "Documents",
    responses((status = 200, body = DocumentResponse), (status = 404))
)]
pub async fn get_document(
    State(state): State<AppState>,
    Path(document_id): Path<Uuid>,
) -> AppResult<Json<DocumentResponse>> {
    Ok(Json(document_response(&state, document_id).await?))
}

#[utoipa::path(
    get,
    path = "/api/documents/{document_id}/events",
    tag = "Documents",
    responses((status = 200, description = "Document processing event stream"), (status = 404))
)]
pub async fn stream_document_events(
    State(state): State<AppState>,
    Path(document_id): Path<Uuid>,
) -> AppResult<Sse<ReceiverStream<Result<Event, Infallible>>>> {
    let initial = document_response(&state, document_id).await?;
    let mut events = state.document_events.subscribe();
    let worker_state = state.clone();
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let mut snapshot = initial;
        loop {
            let terminal = matches!(snapshot.status.as_str(), "ready" | "failed");
            let data = serde_json::to_string(&snapshot).unwrap_or_default();
            if tx
                .send(Ok(Event::default().event("document").data(data)))
                .await
                .is_err()
                || terminal
            {
                return;
            }
            loop {
                match events.recv().await {
                    Ok(id) if id == document_id => break,
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
            match document_response(&worker_state, document_id).await {
                Ok(value) => snapshot = value,
                Err(_) => return,
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

#[utoipa::path(
    get,
    path = "/api/documents/{document_id}/file",
    tag = "Documents",
    responses((status = 200, description = "Original PDF file"), (status = 404))
)]
pub async fn get_document_file(
    State(state): State<AppState>,
    Path(document_id): Path<Uuid>,
) -> AppResult<Response> {
    let (key, file_name): (String, String) =
        sqlx::query_as("SELECT rustfs_key, file_name FROM documents WHERE id = $1")
            .bind(document_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found(format!("document {document_id} was not found")))?;
    let bytes = state.storage.download(&key).await?;
    let safe_name = file_name.replace(['"', '\r', '\n'], "_");
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{safe_name}\""))
            .map_err(|error| AppError::other(error.to_string()))?,
    );
    Ok(response)
}

#[utoipa::path(
    delete,
    path = "/api/documents/{document_id}",
    tag = "Documents",
    responses((status = 200, body = DeleteDocumentResponse), (status = 404))
)]
pub async fn delete_document(
    State(state): State<AppState>,
    Path(document_id): Path<Uuid>,
) -> AppResult<Json<DeleteDocumentResponse>> {
    let (course_id, key): (Uuid, String) =
        sqlx::query_as("SELECT course_id, rustfs_key FROM documents WHERE id = $1")
            .bind(document_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found(format!("document {document_id} was not found")))?;
    let mut transaction = state.pool.begin().await?;
    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(document_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "DELETE FROM semantic_chat_cache WHERE video_id IN (SELECT id FROM videos WHERE course_id = $1)",
    )
    .bind(course_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    if let Err(error) = state.storage.delete(&key).await {
        tracing::warn!(document_id = %document_id, error = %error, "failed to delete PDF object");
    }
    Ok(Json(DeleteDocumentResponse {
        document_id,
        deleted: true,
    }))
}

async fn document_response(state: &AppState, document_id: Uuid) -> AppResult<DocumentResponse> {
    let row: DocumentRow = sqlx::query_as(&format!(
        "{} WHERE d.id = $1 {}",
        document_select(),
        document_order()
    ))
    .bind(document_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found(format!("document {document_id} was not found")))?;
    Ok(row.into())
}

fn document_select() -> &'static str {
    r#"
    SELECT d.id, d.course_id, c.title AS course_title, d.title, d.file_name,
           d.content_type, d.status, d.error_msg, d.page_count,
           COUNT(dc.id)::BIGINT AS chunk_count, d.created_at, d.updated_at
    FROM documents d
    JOIN courses c ON c.id = d.course_id
    LEFT JOIN document_chunks dc ON dc.document_id = d.id
    "#
}

fn document_order() -> &'static str {
    "GROUP BY d.id, c.title ORDER BY d.created_at DESC"
}

impl From<DocumentRow> for DocumentResponse {
    fn from(row: DocumentRow) -> Self {
        Self {
            id: row.id,
            course_id: row.course_id,
            course_title: row.course_title,
            title: row.title,
            file_name: row.file_name,
            content_type: row.content_type,
            status: row.status,
            error_msg: row.error_msg,
            page_count: row.page_count,
            chunk_count: row.chunk_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{document_order, document_select, DocumentRow, MAX_PDF_BYTES};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn document_query_counts_chunks_and_orders_newest_first() {
        assert!(document_select().contains("COUNT(dc.id)::BIGINT AS chunk_count"));
        assert!(document_select().contains("LEFT JOIN document_chunks dc"));
        assert_eq!(
            document_order(),
            "GROUP BY d.id, c.title ORDER BY d.created_at DESC"
        );
    }

    #[test]
    fn document_row_maps_to_response_without_losing_metadata() {
        let now = Utc::now();
        let row = DocumentRow {
            id: Uuid::new_v4(),
            course_id: Uuid::new_v4(),
            course_title: "Course".to_string(),
            title: "Guide".to_string(),
            file_name: "guide.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            status: "ready".to_string(),
            error_msg: None,
            page_count: Some(12),
            chunk_count: 7,
            created_at: now,
            updated_at: now,
        };

        let response: crate::models::DocumentResponse = row.into();

        assert_eq!(response.title, "Guide");
        assert_eq!(response.file_name, "guide.pdf");
        assert_eq!(response.page_count, Some(12));
        assert_eq!(response.chunk_count, 7);
    }

    #[test]
    fn pdf_upload_limit_is_one_hundred_mib() {
        assert_eq!(MAX_PDF_BYTES, 100 * 1024 * 1024);
    }
}
