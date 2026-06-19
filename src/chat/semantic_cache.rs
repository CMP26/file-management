use crate::{models::TranscriptChatSource, AppError, AppResult, AppState};
use std::collections::HashSet;
use uuid::Uuid;

const EMBEDDING_DIMENSIONS: usize = 768;
const LEXICAL_CANDIDATE_LIMIT: i64 = 20;
const VECTOR_ONLY_THRESHOLD: f32 = 0.92;
const STOP_WORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "can", "could", "define", "describe", "do", "does",
    "explain", "for", "give", "how", "i", "in", "is", "it", "know", "lesson", "me", "mean", "of",
    "on", "please", "tell", "the", "this", "to", "video", "what", "which", "who", "why", "would",
    "you",
];

type CacheCandidateRow = (Uuid, String, Option<String>, Vec<String>, f32);

pub(super) struct CacheHit {
    pub id: Uuid,
    pub answer: String,
    pub sources: Vec<TranscriptChatSource>,
    pub similarity: f32,
}

pub(super) async fn lookup(
    state: &AppState,
    video_id: Uuid,
    question: &str,
) -> AppResult<(Vec<f32>, Option<CacheHit>)> {
    let question_embedding = state.embeddings.embed(question).await?;
    let vector = vector_literal(&question_embedding)?;
    let terms = lexical_terms(question);
    let mut transaction = state.pool.begin().await?;
    sqlx::query("SET LOCAL hnsw.iterative_scan = strict_order")
        .execute(&mut *transaction)
        .await?;
    let rows: Vec<CacheCandidateRow> = sqlx::query_as(
        r#"
        WITH candidates AS (
            (
                SELECT id, answer, sources_json, question_terms,
                       (1 - (embedding <=> $3::vector))::REAL AS similarity
                FROM semantic_chat_cache
                WHERE video_id = $1 AND embedding_model = $2
                ORDER BY embedding <=> $3::vector
                LIMIT 1
            )
            UNION
            (
                SELECT id, answer, sources_json, question_terms,
                       (1 - (embedding <=> $3::vector))::REAL AS similarity
                FROM semantic_chat_cache
                WHERE video_id = $1
                  AND embedding_model = $2
                  AND cardinality($4::TEXT[]) > 0
                  AND question_terms && $4::TEXT[]
                ORDER BY embedding <=> $3::vector
                LIMIT $5
            )
        )
        SELECT id, answer, sources_json, question_terms, similarity
        FROM candidates
        "#,
    )
    .bind(video_id)
    .bind(state.embeddings.model())
    .bind(&vector)
    .bind(&terms)
    .bind(LEXICAL_CANDIDATE_LIMIT)
    .fetch_all(&mut *transaction)
    .await?;
    transaction.commit().await?;

    let best = rows
        .into_iter()
        .filter_map(
            |(id, answer, sources_json, cached_terms, vector_similarity)| {
                let lexical_similarity = jaccard_similarity(&terms, &cached_terms);
                let similarity = if lexical_similarity > 0.0 {
                    (vector_similarity * 0.5) + (lexical_similarity * 0.5)
                } else if vector_similarity >= VECTOR_ONLY_THRESHOLD {
                    vector_similarity
                } else {
                    return None;
                };
                Some(CacheHit {
                    id,
                    answer,
                    sources: sources_json
                        .as_deref()
                        .and_then(|value| serde_json::from_str(value).ok())
                        .unwrap_or_default(),
                    similarity,
                })
            },
        )
        .max_by(|left, right| left.similarity.total_cmp(&right.similarity));

    Ok((
        question_embedding,
        best.filter(|hit| hit.similarity >= state.config.semantic_cache_threshold),
    ))
}

pub(super) async fn store(
    state: &AppState,
    video_id: Uuid,
    question: &str,
    embedding: &[f32],
    answer: &str,
    sources: &[TranscriptChatSource],
) -> AppResult<()> {
    let vector = vector_literal(embedding)?;
    let terms = lexical_terms(question);
    let sources_json = if sources.is_empty() {
        None
    } else {
        Some(serde_json::to_string(sources)?)
    };
    sqlx::query(
        r#"
        INSERT INTO semantic_chat_cache
            (video_id, embedding_model, question, question_terms, embedding, answer, sources_json)
        VALUES ($1, $2, $3, $4, $5::vector, $6, $7)
        "#,
    )
    .bind(video_id)
    .bind(state.embeddings.model())
    .bind(question)
    .bind(terms)
    .bind(vector)
    .bind(answer)
    .bind(sources_json)
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub(super) async fn record_hit(state: &AppState, cache_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "UPDATE semantic_chat_cache SET hit_count = hit_count + 1, last_hit_at = now() WHERE id = $1",
    )
    .bind(cache_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn lexical_terms(question: &str) -> Vec<String> {
    let mut terms = question
        .split(|character: char| {
            !character.is_alphanumeric() && character != '+' && character != '#'
        })
        .map(str::to_lowercase)
        .filter(|term| term.len() > 1 && !STOP_WORDS.contains(&term.as_str()))
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}

fn jaccard_similarity(left: &[String], right: &[String]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left = left.iter().collect::<HashSet<_>>();
    let right = right.iter().collect::<HashSet<_>>();
    let intersection = left.intersection(&right).count() as f32;
    let union = left.union(&right).count() as f32;
    intersection / union
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
    let values = embedding
        .iter()
        .map(f32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!("[{values}]"))
}

#[cfg(test)]
mod tests {
    use super::{
        jaccard_similarity, lexical_terms, vector_literal, EMBEDDING_DIMENSIONS,
        VECTOR_ONLY_THRESHOLD,
    };

    #[test]
    fn formats_pgvector_literal_and_validates_dimensions() {
        let embedding = vec![0.25; EMBEDDING_DIMENSIONS];
        let literal = vector_literal(&embedding).expect("valid embedding");
        assert!(literal.starts_with("[0.25,0.25"));
        assert!(literal.ends_with(']'));
        assert!(vector_literal(&[1.0, 2.0]).is_err());
    }

    #[test]
    fn normalizes_short_question_paraphrases() {
        let original = lexical_terms("What is Java?");
        let paraphrase = lexical_terms("Tell me about Java.");
        let specific = lexical_terms("How does Java garbage collection work?");

        assert_eq!(original, vec!["java"]);
        assert_eq!(paraphrase, vec!["java"]);
        assert_eq!(jaccard_similarity(&original, &paraphrase), 1.0);
        assert!(jaccard_similarity(&original, &specific) < 0.5);
    }

    #[test]
    fn rejects_subject_mismatch_despite_high_vector_similarity() {
        let java = lexical_terms("What is Java?");
        let spark = lexical_terms("What are the features of Spark?");
        assert_eq!(jaccard_similarity(&java, &spark), 0.0);

        let vector_similarity = 0.84;
        assert!(vector_similarity < VECTOR_ONLY_THRESHOLD);
    }
}
