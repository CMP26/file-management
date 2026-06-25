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
        let should_flush =
            !current_segments.is_empty() && current_tokens + segment_tokens > target_tokens;

        if should_flush {
            let start_s = current_segments
                .first()
                .map(|item| item.start)
                .unwrap_or(0.0);
            let end_s = current_segments
                .last()
                .map(|item| item.end)
                .unwrap_or(start_s);
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
        let start_s = current_segments
            .first()
            .map(|item| item.start)
            .unwrap_or(0.0);
        let end_s = current_segments
            .last()
            .map(|item| item.end)
            .unwrap_or(start_s);
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

#[cfg(test)]
mod tests {
    use super::chunk_segments;
    use crate::models::TranscriptSegmentInput;

    #[test]
    fn empty_segments_produce_no_chunks() {
        assert!(chunk_segments(&[], 10).is_empty());
    }

    #[test]
    fn chunks_preserve_order_timestamps_and_sequence_indexes() {
        let segments = vec![
            segment(0.0, 1.0, "alpha beta gamma"),
            segment(1.0, 2.0, "delta epsilon zeta"),
            segment(2.0, 3.0, "eta theta iota"),
        ];

        let chunks = chunk_segments(&segments, 5);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].seq_index, 0);
        assert_eq!(chunks[1].seq_index, 1);
        assert_eq!(chunks[2].seq_index, 2);
        assert_eq!(chunks[0].start_s, 0.0);
        assert_eq!(chunks[2].end_s, 3.0);
        assert_eq!(chunks[0].text, "alpha beta gamma");
    }

    #[test]
    fn large_target_groups_segments_into_one_chunk() {
        let segments = vec![segment(0.0, 1.0, "hello"), segment(1.0, 2.0, "world")];

        let chunks = chunk_segments(&segments, 100);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");
        assert_eq!(chunks[0].segments.len(), 2);
    }

    fn segment(start: f64, end: f64, text: &str) -> TranscriptSegmentInput {
        TranscriptSegmentInput {
            start,
            end,
            text: text.to_string(),
        }
    }
}
