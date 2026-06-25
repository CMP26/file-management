use crate::AppResult;
use serde::Deserialize;

pub fn prompt_for_chunk(chunk_text: &str) -> String {
    format!(
        "Given this video transcript segment, create a short topic name. Return ONLY valid JSON, with no markdown and no extra text.\n\nTranscript:\n{chunk_text}\n\nJSON schema:\n{{\n  \"label\": \"short topic name (3-6 words)\"\n}}"
    )
}

#[derive(Debug, Deserialize)]
struct TopicLabelOnly {
    label: String,
}

pub async fn label_chunk(
    client: &crate::llm::gemma::GemmaClient,
    chunk_text: &str,
) -> AppResult<String> {
    let response: TopicLabelOnly = client.generate_json(&prompt_for_chunk(chunk_text)).await?;
    Ok(response.label)
}

#[cfg(test)]
mod tests {
    use super::{label_chunk, prompt_for_chunk};
    use crate::llm::gemma::GemmaClient;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn topic_prompt_contains_transcript_and_schema() {
        let prompt = prompt_for_chunk("spark actions");

        assert!(prompt.contains("spark actions"));
        assert!(prompt.contains("\"label\""));
    }

    #[tokio::test]
    async fn labels_chunk_from_json_response() {
        let body = r#"{"choices":[{"message":{"content":"{\"label\":\"Spark Actions\"}"}}]}"#;
        let client = GemmaClient::new(&start_mock_server(json_response(body)).await, "gemma", 1, 5);

        let label = label_chunk(&client, "spark actions").await.unwrap();

        assert_eq!(label, "Spark Actions");
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
