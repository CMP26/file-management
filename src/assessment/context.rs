use crate::{models::QuestionRecord, AppResult, AppState};

pub async fn transcript_context_for_question(
    state: &AppState,
    question: &QuestionRecord,
) -> AppResult<String> {
    if let Some(topic_id) = question.topic_id {
        let topic_bounds = sqlx::query_as::<_, (f64, f64)>(
            "SELECT start_s, end_s FROM topics WHERE id = $1 AND video_id = $2",
        )
        .bind(topic_id)
        .bind(question.video_id)
        .fetch_optional(&state.pool)
        .await?;

        if let Some((start_s, end_s)) = topic_bounds {
            let segments = sqlx::query_as::<_, (f64, f64, String)>(
                r#"
                SELECT s.start_s, s.end_s, s.text
                FROM transcript_segments s
                JOIN transcripts tr ON tr.id = s.transcript_id
                WHERE tr.video_id = $1
                  AND s.end_s >= $2
                  AND s.start_s <= $3
                ORDER BY s.seq_index
                LIMIT 30
                "#,
            )
            .bind(question.video_id)
            .bind(start_s)
            .bind(end_s)
            .fetch_all(&state.pool)
            .await?;

            if !segments.is_empty() {
                return Ok(segments
                    .into_iter()
                    .map(|(start_s, end_s, text)| format!("[{start_s:.1}s-{end_s:.1}s] {text}"))
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
        }
    }

    let preview = sqlx::query_scalar::<_, String>(
        "SELECT left(full_text, 4000) FROM transcripts WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(question.video_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(preview.unwrap_or_else(|| "Transcript context is not available.".to_string()))
}
