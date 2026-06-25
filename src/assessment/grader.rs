use crate::{
    assessment::context::transcript_context_for_question,
    models::{GradeResponse, QuestionRecord},
    AppResult, AppState,
};
use uuid::Uuid;

pub async fn grade_answer(
    state: &AppState,
    question_id: Uuid,
    user_answer: &str,
) -> AppResult<GradeResponse> {
    let question: QuestionRecord = sqlx::query_as("SELECT * FROM questions WHERE id = $1")
        .bind(question_id)
        .fetch_one(&state.pool)
        .await?;

    if question.question_type.eq_ignore_ascii_case("essay") {
        let transcript_context = transcript_context_for_question(state, &question).await?;
        let prompt = format!(
            "You are a strict academic grader. Use the source video transcript context when judging factual accuracy. Return ONLY valid JSON (no markdown).\n\nQuestion: {}\nSource video transcript context:\n{}\nGrading rubric: {}\nStudent answer: {}\n\nJSON schema:\n{{\n  \"score\": <integer 0-100>,\n  \"is_correct\": <boolean, true if score >= 60>\n}}",
            question.stem,
            transcript_context,
            question.rubric.clone().unwrap_or_default(),
            user_answer
        );
        let result: GradeResponse = state.gemma.generate_json(&prompt).await?;
        return Ok(result);
    }

    if is_completion_question(&question.question_type) {
        let expected_answer = expected_completion_answer(question.rubric.as_deref());
        let is_correct = expected_answer
            .as_deref()
            .map(|expected| {
                normalize_completion_answer(expected) == normalize_completion_answer(user_answer)
            })
            .unwrap_or(false);

        return Ok(GradeResponse {
            score: if is_correct { 100 } else { 0 },
            is_correct,
        });
    }

    let correct_choice: Option<(String,)> = sqlx::query_as(
        "SELECT label FROM choices WHERE question_id = $1 AND is_correct = true LIMIT 1",
    )
    .bind(question_id)
    .fetch_optional(&state.pool)
    .await?;

    let is_correct = correct_choice
        .as_ref()
        .map(|(label,)| label.eq_ignore_ascii_case(user_answer.trim()))
        .unwrap_or(false);

    Ok(GradeResponse {
        score: if is_correct { 100 } else { 0 },
        is_correct,
    })
}

pub(crate) fn is_completion_question(question_type: &str) -> bool {
    matches!(
        question_type.trim().to_ascii_lowercase().as_str(),
        "completion" | "one_word" | "short_answer" | "fill_blank" | "fill-in-the-blank"
    )
}

pub(crate) fn expected_completion_answer(rubric: Option<&str>) -> Option<String> {
    let rubric = rubric?.trim();
    for line in rubric.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("expected answer:") || lower.starts_with("answer:") {
            let (_, answer) = trimmed.split_once(':')?;
            return clean_expected_answer(answer);
        }
    }

    if rubric.split_whitespace().count() <= 3 {
        clean_expected_answer(rubric)
    } else {
        None
    }
}

fn clean_expected_answer(answer: &str) -> Option<String> {
    let cleaned = answer
        .trim()
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '`' | '*' | '.' | ',' | ';' | ':' | '-' | '_'
                )
        })
        .split_whitespace()
        .next()?
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '*' | '.' | ',' | ';' | ':'))
        .to_string();

    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn normalize_completion_answer(answer: &str) -> String {
    answer
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{expected_completion_answer, normalize_completion_answer};

    #[test]
    fn extracts_expected_completion_answer_from_rubric() {
        assert_eq!(
            expected_completion_answer(Some("Expected answer: Java\nMentioned near JVM.")),
            Some("Java".to_string())
        );
        assert_eq!(
            expected_completion_answer(Some("Answer: Spark.")),
            Some("Spark".to_string())
        );
    }

    #[test]
    fn normalizes_completion_answer_case_and_punctuation() {
        assert_eq!(normalize_completion_answer(" Java. "), "java");
        assert_eq!(normalize_completion_answer("JAVA"), "java");
    }
}
