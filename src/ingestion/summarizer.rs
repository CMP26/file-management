use crate::AppResult;

pub fn prompt_for_summary(full_text: &str) -> String {
    format!(
        "Summarize the following video transcript for a student in 1-2 short paragraphs maximum.\nKeep the whole summary under 120 words.\nFocus only on the key concepts, not timestamps.\nReturn only the summary text, with no heading or bullet list.\n\nTranscript:\n{full_text}"
    )
}

pub async fn summarize(
    client: &crate::llm::gemma::GemmaClient,
    full_text: &str,
) -> AppResult<String> {
    client.generate(&prompt_for_summary(full_text)).await
}

#[cfg(test)]
mod tests {
    use super::{prompt_for_summary, summarize};
    use crate::llm::gemma::GemmaClient;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn summary_prompt_contains_constraints_and_transcript() {
        let prompt = prompt_for_summary("full transcript");

        assert!(prompt.contains("under 120 words"));
        assert!(prompt.contains("full transcript"));
    }

    #[tokio::test]
    async fn summarizes_with_gemma_client() {
        let body = r#"{"choices":[{"message":{"content":"Short summary"}}]}"#;
        let client = GemmaClient::new(&start_mock_server(json_response(body)).await, "gemma", 1, 5);

        let summary = summarize(&client, "transcript").await.unwrap();

        assert_eq!(summary, "Short summary");
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
