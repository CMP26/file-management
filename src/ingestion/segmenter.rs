use crate::models::{Chunk, TranscriptSegmentInput};

fn approximate_tokens(text: &str) -> usize {
    let word_count = text.split_whitespace().count();
    (word_count.saturating_mul(13) / 10).max(1)
}

pub fn chunk_segments(segments: &[TranscriptSegmentInput], target_tokens: usize) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut current_segments: Vec<TranscriptSegmentInput> = Vec::new();
    let mut current_text = String::new();
    let mut current_tokens = 0usize;
    let mut seq_index = 0i32;

    for segment in segments {
        let segment_tokens = approximate_tokens(&segment.text);
        let should_flush = !current_segments.is_empty() && current_tokens + segment_tokens > target_tokens;

        if should_flush {
            let start_s = current_segments.first().map(|item| item.start).unwrap_or(0.0);
            let end_s = current_segments.last().map(|item| item.end).unwrap_or(start_s);
            chunks.push(Chunk {
                seq_index,
                start_s,
                end_s,
                text: current_text.trim().to_string(),
                segments: current_segments.clone(),
            });
            seq_index += 1;
            current_segments.clear();
            current_text.clear();
            current_tokens = 0;
        }

        current_tokens += segment_tokens;
        current_text.push_str(&segment.text);
        current_text.push(' ');
        current_segments.push(segment.clone());
    }

    if !current_segments.is_empty() {
        let start_s = current_segments.first().map(|item| item.start).unwrap_or(0.0);
        let end_s = current_segments.last().map(|item| item.end).unwrap_or(start_s);
        chunks.push(Chunk {
            seq_index,
            start_s,
            end_s,
            text: current_text.trim().to_string(),
            segments: current_segments,
        });
    }

    chunks
}
