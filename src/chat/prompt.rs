use super::{DocumentContext, TranscriptSegment};
use crate::models::TranscriptChatMessage;

const CHAT_WINDOW_TURNS: usize = 8;

pub(super) fn build_transcript_chat_prompt(
    video_title: &str,
    _summary: Option<&str>,
    message: &str,
    history: &[TranscriptChatMessage],
    segments: &[TranscriptSegment],
    documents: &[DocumentContext],
) -> String {
    let mut prompt = String::from(
        "You are NexaLearn's learning chat assistant.\n\
        Use the provided video transcript excerpts and course document excerpts when relevant.\n\
        Cite transcript-backed claims inline like [3:25]. Cite document-backed claims like [Document title, p. 4].\n\
        You may also answer general questions that go beyond the video using your broader knowledge.\n\
        When an answer relies on outside knowledge rather than the transcript, say so briefly and avoid inventing video-specific details.\n\
        If the learner asks about the video and the excerpts do not contain enough information, say what is missing and then offer any helpful general context.\n\
        Be concise and helpful.\n\n",
    );

    prompt.push_str(&format!("Video title: {video_title}\n"));

    // The summary is intentionally excluded because transcript excerpts are the source of truth.

    // Transcript context is independent of the chat window and is always present.
    prompt.push_str("Transcript excerpts:\n");
    if segments.is_empty() {
        prompt.push_str("[No transcript excerpts are available for this video.]\n");
    } else {
        for segment in segments {
            prompt.push_str(&format!(
                "[{}] {}\n",
                format_timestamp(segment.start_s),
                truncate_chars(&segment.text, 900)
            ));
        }
    }

    prompt.push_str("\nCourse document excerpts:\n");
    if documents.is_empty() {
        prompt.push_str("[No relevant course document excerpts are available.]\n");
    } else {
        for document in documents {
            let pages = if document.page_start == document.page_end {
                format!("p. {}", document.page_start)
            } else {
                format!("pp. {}-{}", document.page_start, document.page_end)
            };
            prompt.push_str(&format!(
                "[{}, {}] {}\n",
                document.document_title,
                pages,
                truncate_chars(&document.text, 1200)
            ));
        }
    }

    if !history.is_empty() {
        prompt.push_str("Current chat history:\n");
        for chat_message in history.iter().rev().take(CHAT_WINDOW_TURNS).rev() {
            let role = if chat_message.role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            prompt.push_str(role);
            prompt.push_str(": ");
            prompt.push_str(&truncate_chars(&chat_message.content, 700));
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    prompt.push_str("\nLearner question:\n");
    prompt.push_str(message);
    prompt.push_str("\n\nAnswer:");
    prompt
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn format_timestamp(value: f64) -> String {
    let total_seconds = value.max(0.0).floor() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_only_prompt_history_and_keeps_transcript() {
        let history = (0..12)
            .map(|index| TranscriptChatMessage {
                role: if index % 2 == 0 {
                    "user".to_string()
                } else {
                    "assistant".to_string()
                },
                content: format!("message-{index}"),
            })
            .collect::<Vec<_>>();
        let segments = vec![TranscriptSegment {
            seq_index: 0,
            start_s: 12.0,
            end_s: 15.0,
            text: "transcript-context".to_string(),
        }];

        let prompt =
            build_transcript_chat_prompt("Lesson", None, "question", &history, &segments, &[]);
        let prompt_lines = prompt.lines().collect::<Vec<_>>();

        for index in 0..4 {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            let line = format!("{role}: message-{index}");
            assert!(!prompt_lines.contains(&line.as_str()));
        }
        for index in 4..12 {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            let line = format!("{role}: message-{index}");
            assert!(prompt_lines.contains(&line.as_str()));
        }
        assert!(prompt.contains("[0:12] transcript-context"));
        assert!(prompt.contains("Learner question:\nquestion"));
    }

    #[test]
    fn keeps_transcript_section_when_no_segments_exist() {
        let prompt = build_transcript_chat_prompt("Lesson", None, "question", &[], &[], &[]);

        assert!(prompt.contains("Transcript excerpts:"));
        assert!(prompt.contains("No transcript excerpts are available"));
    }

    #[test]
    fn includes_document_context_with_page_citation() {
        let documents = vec![DocumentContext {
            document_id: uuid::Uuid::new_v4(),
            document_title: "Java Reference".to_string(),
            seq_index: 0,
            page_start: 4,
            page_end: 4,
            text: "The JVM executes Java bytecode.".to_string(),
        }];

        let prompt = build_transcript_chat_prompt("Lesson", None, "question", &[], &[], &documents);

        assert!(prompt.contains("[Java Reference, p. 4] The JVM executes Java bytecode."));
        assert!(prompt.contains("No transcript excerpts are available"));
    }
}
