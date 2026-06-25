use crate::{
    ingestion::{
        audio::{create_playback_video, extract_audio},
        question_gen::generate_essay_questions,
        segmenter::chunk_segments,
        summarizer::summarize,
        topic_labeler::label_chunk,
    },
    models::{GeneratedQuestion, TranscriptSegmentInput},
    AppResult, AppState,
};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VideoProcessStage {
    ExtractingAudio,
    LabelingTopics,
    GeneratingQuestions,
    Summarizing,
}

impl VideoProcessStage {
    pub fn as_status(self) -> &'static str {
        match self {
            Self::ExtractingAudio => "extracting_audio",
            Self::LabelingTopics => "labeling_topics",
            Self::GeneratingQuestions => "generating_questions",
            Self::Summarizing => "summarizing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" | "extracting_audio" | "transcribing" => Some(Self::ExtractingAudio),
            "labeling_topics" => Some(Self::LabelingTopics),
            "generating_questions" => Some(Self::GeneratingQuestions),
            "summarizing" => Some(Self::Summarizing),
            _ => None,
        }
    }
}

pub async fn process_video(state: AppState, video_id: Uuid) -> AppResult<()> {
    process_video_from_stage(state, video_id, VideoProcessStage::ExtractingAudio).await
}

pub async fn process_video_from_stage(
    state: AppState,
    video_id: Uuid,
    stage: VideoProcessStage,
) -> AppResult<()> {
    match process_video_inner(state.clone(), video_id, stage).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            let _ = update_status(&state, video_id, "failed", Some(&message)).await;
            Err(error)
        }
    }
}

pub async fn prepare_video_recovery(
    state: &AppState,
    video_id: Uuid,
    stage: VideoProcessStage,
) -> AppResult<()> {
    let mut transaction = state.pool.begin().await?;
    match stage {
        VideoProcessStage::ExtractingAudio => {
            sqlx::query("DELETE FROM transcripts WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM topics WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM questions WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
        VideoProcessStage::LabelingTopics => {
            sqlx::query("DELETE FROM questions WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM topics WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
        VideoProcessStage::GeneratingQuestions => {
            sqlx::query("DELETE FROM questions WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
        VideoProcessStage::Summarizing => {
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
    }
    sqlx::query("DELETE FROM semantic_chat_cache WHERE video_id = $1")
        .bind(video_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE videos SET status = $1, error_msg = NULL WHERE id = $2")
        .bind(stage.as_status())
        .bind(video_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    let _ = state.video_events.send(video_id);
    Ok(())
}

pub async fn infer_video_recovery_stage(
    state: &AppState,
    video_id: Uuid,
    status: &str,
) -> AppResult<VideoProcessStage> {
    if let Some(stage) = VideoProcessStage::parse(status) {
        if stage == VideoProcessStage::ExtractingAudio {
            return Ok(stage);
        }
    }

    let transcript_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM transcripts WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&state.pool)
            .await?;
    if transcript_count == 0 {
        return Ok(VideoProcessStage::ExtractingAudio);
    }

    let topic_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM topics WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&state.pool)
            .await?;
    if topic_count == 0 {
        return Ok(VideoProcessStage::LabelingTopics);
    }

    let question_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM questions WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&state.pool)
            .await?;
    if question_count == 0 {
        return Ok(VideoProcessStage::GeneratingQuestions);
    }

    Ok(VideoProcessStage::Summarizing)
}

async fn process_video_inner(
    state: AppState,
    video_id: Uuid,
    start_stage: VideoProcessStage,
) -> AppResult<()> {
    let tmp_dir = PathBuf::from(&state.config.tmp_dir);
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let original_key: String = sqlx::query_scalar("SELECT rustfs_key FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_one(&state.pool)
        .await?;

    let video_path = tmp_dir.join(format!("{video_id}.mp4"));
    let playback_path = tmp_dir.join(format!("{video_id}.playback.mp4"));
    let wav_path = tmp_dir.join(format!("{video_id}.wav"));

    let transcript = if start_stage <= VideoProcessStage::ExtractingAudio {
        update_status(&state, video_id, "extracting_audio", None).await?;
        let video_bytes = state.storage.download(&original_key).await?;
        tokio::fs::write(&video_path, video_bytes).await?;
        match create_playback_video(&video_path, &playback_path).await {
            Ok(()) => {
                let playback_bytes = tokio::fs::read(&playback_path).await?;
                state
                    .storage
                    .upload(
                        &format!("videos/{video_id}/playback.mp4"),
                        playback_bytes,
                        "video/mp4",
                    )
                    .await?;
            }
            Err(error) => {
                tracing::warn!(video_id = %video_id, error = %error, "failed to create browser playback video");
            }
        }
        extract_audio(&video_path, &wav_path).await?;

        update_status(&state, video_id, "transcribing", None).await?;
        let transcript = state.whisper.transcribe(&wav_path, "en").await?;
        insert_transcript(&state, video_id, &transcript).await?;
        upload_transcript_artifacts(&state, video_id, &transcript).await?;
        transcript
    } else {
        load_transcript(&state, video_id).await?
    };

    let topic_records = if start_stage <= VideoProcessStage::LabelingTopics {
        update_status(&state, video_id, "labeling_topics", None).await?;
        label_topics(&state, video_id, &transcript.segments).await?
    } else {
        load_topic_records(&state, video_id).await?
    };

    if start_stage <= VideoProcessStage::GeneratingQuestions {
        update_status(&state, video_id, "generating_questions", None).await?;
        generate_and_insert_questions(&state, video_id, &transcript.full_text, &topic_records)
            .await?;
    }

    if start_stage <= VideoProcessStage::Summarizing {
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
    }

    update_status(&state, video_id, "ready", None).await?;

    let _ = tokio::fs::remove_file(video_path).await;
    let _ = tokio::fs::remove_file(playback_path).await;
    let _ = tokio::fs::remove_file(wav_path).await;

    Ok(())
}

async fn insert_transcript(
    state: &AppState,
    video_id: Uuid,
    transcript: &crate::models::TranscribeResponse,
) -> AppResult<()> {
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

    Ok(())
}

async fn upload_transcript_artifacts(
    state: &AppState,
    video_id: Uuid,
    transcript: &crate::models::TranscribeResponse,
) -> AppResult<()> {
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
    Ok(())
}

async fn load_transcript(
    state: &AppState,
    video_id: Uuid,
) -> AppResult<crate::models::TranscribeResponse> {
    let (transcript_id, full_text): (Uuid, String) =
        sqlx::query_as("SELECT id, full_text FROM transcripts WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1")
            .bind(video_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| crate::AppError::external("cannot resume video processing without a saved transcript"))?;
    let segments = sqlx::query_as::<_, (f64, f64, String)>(
        r#"
        SELECT start_s, end_s, text
        FROM transcript_segments
        WHERE transcript_id = $1
        ORDER BY seq_index ASC
        "#,
    )
    .bind(transcript_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(start, end, text)| TranscriptSegmentInput { start, end, text })
    .collect::<Vec<_>>();

    Ok(crate::models::TranscribeResponse {
        full_text,
        segments,
    })
}

async fn label_topics(
    state: &AppState,
    video_id: Uuid,
    segments: &[TranscriptSegmentInput],
) -> AppResult<Vec<(Uuid, String, crate::models::Chunk)>> {
    let chunks = chunk_segments(segments, 300);
    let mut topic_records = Vec::new();

    for chunk in &chunks {
        let label = label_chunk(&state.gemma, &chunk.text).await?;

        let topic_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO topics (video_id, label, start_s, end_s, seq_order)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
        )
        .bind(video_id)
        .bind(&label)
        .bind(chunk.start_s)
        .bind(chunk.end_s)
        .bind(chunk.seq_index)
        .fetch_one(&state.pool)
        .await?;

        topic_records.push((topic_id, label, chunk.clone()));
    }

    if topic_records.is_empty() {
        return Err(crate::AppError::external(
            "no transcript chunks were available for topic/question generation",
        ));
    }
    Ok(topic_records)
}

async fn load_topic_records(
    state: &AppState,
    video_id: Uuid,
) -> AppResult<Vec<(Uuid, String, crate::models::Chunk)>> {
    let records = sqlx::query_as::<_, (Uuid, String, f64, f64, i32)>(
        r#"
        SELECT id, label, start_s, end_s, seq_order
        FROM topics
        WHERE video_id = $1
        ORDER BY seq_order ASC
        "#,
    )
    .bind(video_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(topic_id, label, start_s, end_s, seq_order)| {
        (
            topic_id,
            label.clone(),
            crate::models::Chunk {
                seq_index: seq_order,
                start_s,
                end_s,
                text: label,
                segments: Vec::new(),
            },
        )
    })
    .collect::<Vec<_>>();

    if records.is_empty() {
        return Err(crate::AppError::external(
            "cannot resume question generation without saved topics",
        ));
    }
    Ok(records)
}

async fn generate_and_insert_questions(
    state: &AppState,
    video_id: Uuid,
    transcript_text: &str,
    topic_records: &[(Uuid, String, crate::models::Chunk)],
) -> AppResult<()> {
    let topic_context = topic_records
        .iter()
        .map(|(_, label, chunk)| {
            format!(
                "- {} ({}s-{}s)",
                label,
                chunk.start_s.round() as i64,
                chunk.end_s.round() as i64
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let question_count = topic_records.len();
    let generated_questions = generate_essay_questions(
        &state.gemma,
        &topic_context,
        transcript_text,
        question_count,
    )
    .await?;
    let generated_questions = normalize_essay_questions(generated_questions, question_count)?;
    insert_topic_questions(state, video_id, topic_records, generated_questions).await?;
    Ok(())
}

async fn update_status(
    state: &AppState,
    video_id: Uuid,
    status: &str,
    error_msg: Option<&str>,
) -> AppResult<()> {
    tracing::info!(video_id = %video_id, status = %status, error_msg = error_msg, "video processing status updated");
    sqlx::query("UPDATE videos SET status = $1, error_msg = $2 WHERE id = $3")
        .bind(status)
        .bind(error_msg)
        .bind(video_id)
        .execute(&state.pool)
        .await?;
    let _ = state.video_events.send(video_id);
    Ok(())
}

fn normalize_essay_questions(
    questions: Vec<GeneratedQuestion>,
    expected_count: usize,
) -> AppResult<Vec<GeneratedQuestion>> {
    if questions.len() != expected_count {
        return Err(crate::AppError::external(format!(
            "expected exactly {expected_count} essay questions, got {}",
            questions.len(),
        )));
    }

    Ok(questions
        .into_iter()
        .map(|mut question| {
            question.question_type = "essay".to_string();
            question.choices = None;
            if question.difficulty.trim().is_empty() {
                question.difficulty = "medium".to_string();
            }
            if question.rubric.as_deref().unwrap_or_default().trim().is_empty() {
                question.rubric = Some("Grade for conceptual accuracy, use of relevant details from the video, and clarity of explanation.".to_string());
            }
            question
        })
        .collect())
}

async fn insert_topic_questions(
    state: &AppState,
    video_id: Uuid,
    topic_records: &[(Uuid, String, crate::models::Chunk)],
    questions: Vec<GeneratedQuestion>,
) -> AppResult<()> {
    for (index, question) in questions.into_iter().enumerate() {
        let topic_id = topic_records.get(index).map(|(topic_id, _, _)| *topic_id);
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
