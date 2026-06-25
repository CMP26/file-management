use crate::{models::GeneratedQuestion, AppResult};

pub fn prompt_for_assessment_questions(
    topic_label: &str,
    transcript_text: &str,
    question_count: usize,
    questions_per_topic: usize,
) -> String {
    format!(
        "Create assessment questions for this single topic.\n\nTopic: {topic_label}\n\nTranscript excerpt:\n{transcript_text}\n\nReturn ONLY a valid JSON array with exactly {question_count} items. No markdown. Use exactly {questions_per_topic} questions for this topic:\n1. easy mcq with labels A, B, C, D and one correct choice\n2. medium true_false with labels T and F, text True and False, and one correct choice\n3. hard completion asking for exactly one word, choices null, rubric starts with \"Expected answer: <word>\"\n\nUse this compact schema:\n[\n  {{\"stem\":\"short question\",\"question_type\":\"mcq|true_false|completion\",\"difficulty\":\"easy|medium|hard\",\"choices\":[{{\"label\":\"A\",\"text\":\"choice\",\"is_correct\":true}}],\"rubric\":null}}\n]\n\nFor mcq and true_false set rubric to null. Keep stems short. Use only facts supported by the transcript excerpt."
    )
}

pub async fn generate_assessment_questions(
    client: &crate::llm::gemma::GemmaClient,
    topic_label: &str,
    transcript_text: &str,
    question_count: usize,
    questions_per_topic: usize,
) -> AppResult<Vec<GeneratedQuestion>> {
    client
        .generate_json(&prompt_for_assessment_questions(
            topic_label,
            transcript_text,
            question_count,
            questions_per_topic,
        ))
        .await
}

#[cfg(test)]
mod tests {
    use super::{generate_assessment_questions, prompt_for_assessment_questions};
    use crate::llm::gemma::GemmaClient;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn assessment_question_prompt_favors_objective_questions() {
        let prompt = prompt_for_assessment_questions("Topic A", "Transcript text", 3, 3);

        assert!(prompt.contains("Topic A"));
        assert!(prompt.contains("Transcript text"));
        assert!(prompt.contains("exactly 3 items"));
        assert!(prompt.contains("easy mcq"));
        assert!(prompt.contains("medium true_false"));
        assert!(prompt.contains("Expected answer: <word>"));
        assert!(prompt.contains("For mcq and true_false set rubric to null"));
    }

    #[tokio::test]
    async fn generates_assessment_questions_from_json_response() {
        let body = r#"{"choices":[{"message":{"content":"[{\"stem\":\"What runs Spark code?\",\"question_type\":\"mcq\",\"difficulty\":\"easy\",\"rubric\":\"Select the runtime.\",\"choices\":[{\"label\":\"A\",\"text\":\"JVM\",\"is_correct\":true},{\"label\":\"B\",\"text\":\"Browser\",\"is_correct\":false}]}]"}}]}"#;
        let client = GemmaClient::new(&start_mock_server(json_response(body)).await, "gemma", 1, 5);

        let questions = generate_assessment_questions(&client, "Spark", "Transcript", 1, 1)
            .await
            .unwrap();

        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].stem, "What runs Spark code?");
        assert_eq!(questions[0].question_type, "mcq");
        assert_eq!(questions[0].choices.as_ref().unwrap()[0].label, "A");
    }

    async fn start_mock_server(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn json_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        )
    }
}
