use crate::{models::GeneratedQuestion, AppResult};

pub fn prompt_for_essay_questions(
    topic_context: &str,
    transcript_text: &str,
    question_count: usize,
) -> String {
    format!(
        "You are creating an assessment for a learning platform.\n\nTopics:\n{topic_context}\n\nTranscript:\n{transcript_text}\n\nGenerate exactly {question_count} essay questions total: one essay question for each topic, in the same order as the Topics list. Return ONLY valid JSON array, with no markdown and no extra text. Every item must use this exact schema:\n[\n  {{\n    \"stem\": \"essay question text\",\n    \"question_type\": \"essay\",\n    \"difficulty\": \"medium\",\n    \"rubric\": \"specific grading guidance for this essay question\",\n    \"choices\": null\n  }}\n]\n\nRules:\n- Generate only essay questions.\n- Do not generate MCQ or true/false questions.\n- Return exactly {question_count} items.\n- Each question should assess its matching topic."
    )
}

pub async fn generate_essay_questions(
    client: &crate::llm::gemma::GemmaClient,
    topic_context: &str,
    transcript_text: &str,
    question_count: usize,
) -> AppResult<Vec<GeneratedQuestion>> {
    client
        .generate_json(&prompt_for_essay_questions(
            topic_context,
            transcript_text,
            question_count,
        ))
        .await
}

#[cfg(test)]
mod tests {
    use super::{generate_essay_questions, prompt_for_essay_questions};
    use crate::llm::gemma::GemmaClient;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn essay_question_prompt_includes_topics_transcript_and_count() {
        let prompt = prompt_for_essay_questions("Topic A", "Transcript text", 3);

        assert!(prompt.contains("Topic A"));
        assert!(prompt.contains("Transcript text"));
        assert!(prompt.contains("Generate exactly 3 essay questions"));
        assert!(prompt.contains("\"question_type\": \"essay\""));
    }

    #[tokio::test]
    async fn generates_essay_questions_from_json_response() {
        let body = r#"{"choices":[{"message":{"content":"[{\"stem\":\"Explain Spark\",\"question_type\":\"essay\",\"difficulty\":\"medium\",\"rubric\":\"Mention transformations\",\"choices\":null}]"}}]}"#;
        let client = GemmaClient::new(&start_mock_server(json_response(body)).await, "gemma", 1, 5);

        let questions = generate_essay_questions(&client, "Spark", "Transcript", 1)
            .await
            .unwrap();

        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].stem, "Explain Spark");
        assert_eq!(questions[0].question_type, "essay");
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
