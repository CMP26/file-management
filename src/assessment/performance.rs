pub fn score_percent(total_score: i32, graded_count: i64) -> Option<f64> {
    if graded_count <= 0 {
        None
    } else {
        Some((f64::from(total_score) / graded_count as f64).clamp(0.0, 100.0))
    }
}

pub fn category_for_score(score_percent: f64) -> &'static str {
    match score_percent {
        score if score >= 90.0 => "Excellent",
        score if score >= 80.0 => "Very Good",
        score if score >= 70.0 => "Good",
        score if score >= 60.0 => "Accepted",
        _ => "Fail",
    }
}

pub fn category_for_optional_score(score_percent: Option<f64>) -> Option<String> {
    score_percent.map(|score| category_for_score(score).to_string())
}

#[cfg(test)]
mod tests {
    use super::{category_for_score, score_percent};

    #[test]
    fn maps_scores_to_student_facing_categories() {
        assert_eq!(category_for_score(95.0), "Excellent");
        assert_eq!(category_for_score(82.0), "Very Good");
        assert_eq!(category_for_score(75.0), "Good");
        assert_eq!(category_for_score(60.0), "Accepted");
        assert_eq!(category_for_score(59.9), "Fail");
    }

    #[test]
    fn averages_scores_per_graded_answer() {
        assert_eq!(score_percent(180, 2), Some(90.0));
        assert_eq!(score_percent(0, 0), None);
    }
}
