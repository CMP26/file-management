use crate::{
    ingestion::{audio::extract_audio, question_gen::generate_questions, segmenter::chunk_segments, summarizer::summarize, topic_labeler::label_chunk},
    models::{GeneratedChoice, GeneratedQuestion, TranscriptSegmentInput, TopicLabelResponse},
    AppResult, AppState,
};
use std::path::PathBuf;
use uuid::Uuid;

pub async fn process_video(state: AppState, video_id: Uuid) -> AppResult<()> {
    let tmp_dir = PathBuf::from(&state.config.tmp_dir);
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let original_key: String = sqlx::query_scalar("SELECT rustfs_key FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_one(&state.pool)
        .await?;

    let video_path = tmp_dir.join(format!("{video_id}.mp4"));
    let wav_path = tmp_dir.join(format!("{video_id}.wav"));

    update_status(&state, video_id, "extracting_audio", None).await?;
    let video_bytes = state.storage.download(&original_key).await?;
    tokio::fs::write(&video_path, video_bytes).await?;
    extract_audio(&video_path, &wav_path).await?;

    update_status(&state, video_id, "transcribing", None).await?;
    let transcript = state.whisper.transcribe(&wav_path, "en").await?;

    let transcript_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO transcripts (video_id, full_text, language)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(video_id)
    .bind(&transcript.full_text)
    .bind("en")
    .fetch_one(&state.pool)
    .await?;

    for (seq_index, segment) in transcript.segments.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO transcript_segments (transcript_id, seq_index, start_s, end_s, text)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(transcript_id)
        .bind(seq_index as i32)
        .bind(segment.start)
        .bind(segment.end)
        .bind(&segment.text)
        .execute(&state.pool)
        .await?;
    }

    state
        .storage
        .upload(
            &format!("videos/{video_id}/transcript.txt"),
            transcript.full_text.as_bytes().to_vec(),
            "text/plain; charset=utf-8",
        )
        .await?;

    let vtt_text = transcript_to_vtt(&transcript.segments);
    state
        .storage
        .upload(
            &format!("videos/{video_id}/transcript.vtt"),
            vtt_text.into_bytes(),
            "text/vtt; charset=utf-8",
        )
        .await?;

    update_status(&state, video_id, "labeling_topics", None).await?;
    let chunks = chunk_segments(&transcript.segments, 300);
    let mut topic_records = Vec::new();

    for chunk in &chunks {
        let label: TopicLabelResponse = match label_chunk(&state.gemma, &chunk.text).await {
            Ok(value) => value,
            Err(_) => TopicLabelResponse {
                label: format!("Topic {}", chunk.seq_index + 1),
                start_s: chunk.start_s,
                end_s: chunk.end_s,
            },
        };

        let topic_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO topics (video_id, label, start_s, end_s, seq_order)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
        )
        .bind(video_id)
        .bind(&label.label)
        .bind(label.start_s)
        .bind(label.end_s)
        .bind(chunk.seq_index)
        .fetch_one(&state.pool)
        .await?;

        topic_records.push((topic_id, label, chunk.clone()));
    }

    update_status(&state, video_id, "generating_questions", None).await?;
    for (topic_id, label, chunk) in topic_records {
        let generated_questions = match generate_questions(&state.gemma, &label.label, &chunk.text, 3).await {
            Ok(items) => items,
            Err(_) => fallback_questions(&label.label),
        };

        insert_questions(&state, video_id, topic_id, generated_questions).await?;
    }

    update_status(&state, video_id, "summarizing", None).await?;
    let summary = summarize(&state.gemma, &transcript.full_text)
        .await
        .unwrap_or_else(|_| "Summary unavailable in mock mode.".to_string());

    sqlx::query(
        r#"
        INSERT INTO summaries (video_id, content)
        VALUES ($1, $2)
        "#,
    )
    .bind(video_id)
    .bind(&summary)
    .execute(&state.pool)
    .await?;

    update_status(&state, video_id, "ready", None).await?;

    let _ = tokio::fs::remove_file(video_path).await;
    let _ = tokio::fs::remove_file(wav_path).await;

    Ok(())
}

async fn update_status(state: &AppState, video_id: Uuid, status: &str, error_msg: Option<&str>) -> AppResult<()> {
    sqlx::query("UPDATE videos SET status = $1, error_msg = $2 WHERE id = $3")
        .bind(status)
        .bind(error_msg)
        .bind(video_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

fn fallback_questions(topic_label: &str) -> Vec<GeneratedQuestion> {
    vec![
        GeneratedQuestion {
            stem: format!("What is the main idea of {topic_label}?"),
            question_type: "mcq".to_string(),
            difficulty: "easy".to_string(),
            rubric: None,
            choices: Some(vec![
                GeneratedChoice { label: "A".to_string(), text: "It describes the topic broadly.".to_string(), is_correct: true },
                GeneratedChoice { label: "B".to_string(), text: "It is unrelated to the topic.".to_string(), is_correct: false },
            ]),
        },
        GeneratedQuestion {
            stem: format!("Explain one detail from {topic_label} in your own words."),
            question_type: "essay".to_string(),
            difficulty: "medium".to_string(),
            rubric: Some("Mention the key concept and one supporting detail.".to_string()),
            choices: None,
        },
    ]
}

async fn insert_questions(state: &AppState, video_id: Uuid, topic_id: Uuid, questions: Vec<GeneratedQuestion>) -> AppResult<()> {
    for question in questions {
        let question_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO questions (video_id, topic_id, stem, question_type, difficulty, rubric)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(video_id)
        .bind(topic_id)
        .bind(&question.stem)
        .bind(&question.question_type)
        .bind(&question.difficulty)
        .bind(&question.rubric)
        .fetch_one(&state.pool)
        .await?;

        if let Some(choices) = question.choices {
            for choice in choices {
                sqlx::query(
                    r#"
                    INSERT INTO choices (question_id, label, text, is_correct)
                    VALUES ($1, $2, $3, $4)
                    "#,
                )
                .bind(question_id)
                .bind(choice.label)
                .bind(choice.text)
                .bind(choice.is_correct)
                .execute(&state.pool)
                .await?;
            }
        }
    }

    Ok(())
}

fn transcript_to_vtt(segments: &[TranscriptSegmentInput]) -> String {
    let mut output = String::from("WEBVTT\n\n");
    for (index, segment) in segments.iter().enumerate() {
        output.push_str(&format!(
            "{index}\n{} --> {}\n{}\n\n",
            format_timestamp(segment.start),
            format_timestamp(segment.end),
            segment.text
        ));
    }
    output
}

fn format_timestamp(seconds: f64) -> String {
    let total_millis = (seconds * 1000.0).round() as u64;
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let secs = (total_millis % 60_000) / 1000;
    let millis = total_millis % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}")
}
