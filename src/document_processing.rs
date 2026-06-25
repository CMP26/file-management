use crate::{AppError, AppResult, AppState};
use std::path::PathBuf;
use tokio::process::Command;
use uuid::Uuid;

const TARGET_CHUNK_WORDS: usize = 220;
const CHUNK_OVERLAP_WORDS: usize = 40;
const EMBEDDING_DIMENSIONS: usize = 768;

#[derive(Debug, Clone)]
struct DocumentChunk {
    seq_index: i32,
    page_start: i32,
    page_end: i32,
    content: String,
}

pub async fn process_document(state: AppState, document_id: Uuid) -> AppResult<()> {
    match process_document_inner(state.clone(), document_id).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            let _ = update_status(&state, document_id, "failed", Some(&message)).await;
            Err(error)
        }
    }
}

async fn process_document_inner(state: AppState, document_id: Uuid) -> AppResult<()> {
    let tmp_dir = PathBuf::from(&state.config.tmp_dir);
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let (course_id, rustfs_key): (Uuid, String) =
        sqlx::query_as("SELECT course_id, rustfs_key FROM documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(&state.pool)
            .await?;
    let pdf_path = tmp_dir.join(format!("{document_id}.pdf"));

    update_status(&state, document_id, "extracting", None).await?;
    let pdf_bytes = state.storage.download(&rustfs_key).await?;
    tokio::fs::write(&pdf_path, pdf_bytes).await?;

    let output = Command::new("pdftotext")
        .arg("-layout")
        .arg(&pdf_path)
        .arg("-")
        .output()
        .await
        .map_err(|error| AppError::external(format!("failed to start pdftotext: {error}")))?;
    if !output.status.success() {
        return Err(AppError::external(format!(
            "pdftotext failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let extracted = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let mut pages = extracted
        .split('\u{000c}')
        .map(clean_page_text)
        .collect::<Vec<_>>();
    while pages.last().is_some_and(|page| page.is_empty()) {
        pages.pop();
    }
    let page_count = pages.len() as i32;
    let full_text = pages
        .iter()
        .filter(|page| !page.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");
    if full_text.trim().is_empty() {
        return Err(AppError::bad_request(
            "the PDF contains no extractable text; scanned PDFs require OCR before upload",
        ));
    }

    let chunks = chunk_pages(&pages, TARGET_CHUNK_WORDS, CHUNK_OVERLAP_WORDS);
    if chunks.is_empty() {
        return Err(AppError::external(
            "no document chunks were produced from the extracted PDF text",
        ));
    }

    update_status(&state, document_id, "embedding", None).await?;
    let mut embedded_chunks = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let embedding = state.embeddings.embed_document(&chunk.content).await?;
        let vector = vector_literal(&embedding)?;
        embedded_chunks.push((chunk, vector));
    }

    let mut transaction = state.pool.begin().await?;
    sqlx::query("DELETE FROM document_chunks WHERE document_id = $1")
        .bind(document_id)
        .execute(&mut *transaction)
        .await?;
    for (chunk, vector) in embedded_chunks {
        sqlx::query(
            r#"
            INSERT INTO document_chunks
                (document_id, seq_index, page_start, page_end, content, embedding_model, embedding)
            VALUES ($1, $2, $3, $4, $5, $6, $7::vector)
            "#,
        )
        .bind(document_id)
        .bind(chunk.seq_index)
        .bind(chunk.page_start)
        .bind(chunk.page_end)
        .bind(chunk.content)
        .bind(state.embeddings.model())
        .bind(vector)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        r#"
        UPDATE documents
        SET full_text = $1, page_count = $2, status = 'ready',
            error_msg = NULL, updated_at = now()
        WHERE id = $3
        "#,
    )
    .bind(full_text)
    .bind(page_count)
    .bind(document_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"
        DELETE FROM semantic_chat_cache
        WHERE video_id IN (SELECT id FROM videos WHERE course_id = $1)
        "#,
    )
    .bind(course_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    let _ = state.document_events.send(document_id);
    let _ = tokio::fs::remove_file(pdf_path).await;
    Ok(())
}

async fn update_status(
    state: &AppState,
    document_id: Uuid,
    status: &str,
    error_msg: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE documents SET status = $1, error_msg = $2, updated_at = now() WHERE id = $3",
    )
    .bind(status)
    .bind(error_msg)
    .bind(document_id)
    .execute(&state.pool)
    .await?;
    let _ = state.document_events.send(document_id);
    Ok(())
}

fn clean_page_text(page: &str) -> String {
    page.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn chunk_pages(pages: &[String], target_words: usize, overlap_words: usize) -> Vec<DocumentChunk> {
    let mut chunks = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let words = page.split_whitespace().collect::<Vec<_>>();
        if words.is_empty() {
            continue;
        }
        let mut start = 0usize;
        while start < words.len() {
            let end = (start + target_words).min(words.len());
            chunks.push(DocumentChunk {
                seq_index: chunks.len() as i32,
                page_start: page_index as i32 + 1,
                page_end: page_index as i32 + 1,
                content: words[start..end].join(" "),
            });
            if end == words.len() {
                break;
            }
            start = end.saturating_sub(overlap_words.min(target_words.saturating_sub(1)));
        }
    }
    chunks
}

fn vector_literal(embedding: &[f32]) -> AppResult<String> {
    if embedding.len() != EMBEDDING_DIMENSIONS {
        return Err(AppError::external(format!(
            "embedding model returned {} dimensions; expected {EMBEDDING_DIMENSIONS}",
            embedding.len()
        )));
    }
    if embedding.iter().any(|value| !value.is_finite()) {
        return Err(AppError::external(
            "embedding model returned a non-finite value",
        ));
    }
    Ok(format!(
        "[{}]",
        embedding
            .iter()
            .map(f32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        chunk_pages, clean_page_text, vector_literal, CHUNK_OVERLAP_WORDS, TARGET_CHUNK_WORDS,
    };
    use crate::AppError;

    #[test]
    fn chunks_pdf_pages_without_losing_page_numbers() {
        let pages = vec![
            (0..12)
                .map(|index| format!("a{index}"))
                .collect::<Vec<_>>()
                .join(" "),
            "second page words".to_string(),
        ];
        let chunks = chunk_pages(&pages, 5, 2);

        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[3].page_start, 1);
        assert_eq!(chunks[4].page_start, 2);
        assert!(chunks[1].content.starts_with("a3 a4"));
    }

    #[test]
    fn clean_page_text_trims_trailing_space_and_outer_blank_lines() {
        let cleaned = clean_page_text("\n  title  \nbody text   \n\n");

        assert_eq!(cleaned, "title\nbody text");
    }

    #[test]
    fn chunk_pages_skips_empty_pages_and_uses_default_overlap() {
        let page = (0..(TARGET_CHUNK_WORDS + 20))
            .map(|index| format!("w{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = chunk_pages(
            &["".to_string(), page],
            TARGET_CHUNK_WORDS,
            CHUNK_OVERLAP_WORDS,
        );

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].page_start, 2);
        assert!(chunks[1]
            .content
            .starts_with(&format!("w{}", TARGET_CHUNK_WORDS - CHUNK_OVERLAP_WORDS)));
    }

    #[test]
    fn vector_literal_validates_dimensions_and_finiteness() {
        let vector = vector_literal(&vec![0.5; 768]).unwrap();
        assert!(vector.starts_with("[0.5,0.5"));

        assert!(matches!(vector_literal(&[0.1]), Err(AppError::External(_))));

        let mut invalid = vec![0.0; 768];
        invalid[0] = f32::INFINITY;
        assert!(matches!(
            vector_literal(&invalid),
            Err(AppError::External(_))
        ));
    }
}
