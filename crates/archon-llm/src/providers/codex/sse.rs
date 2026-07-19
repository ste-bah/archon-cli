use futures_util::{Stream, StreamExt};

use crate::provider::LlmError;

use super::types::ResponseStreamEvent;

pub fn parse_codex_sse_frame(frame: &str) -> Vec<Result<ResponseStreamEvent, LlmError>> {
    let mut data = String::new();
    for line in frame.lines().map(str::trim_end) {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(part) = line.strip_prefix("data:") {
            let part = part.trim_start();
            if part == "[DONE]" {
                return Vec::new();
            }
            data.push_str(part);
        }
    }

    if data.trim().is_empty() {
        return Vec::new();
    }

    match serde_json::from_str::<ResponseStreamEvent>(&data) {
        Ok(event) => vec![Ok(event)],
        Err(e) => {
            tracing::warn!("skipping malformed Codex SSE event: {e}");
            Vec::new()
        }
    }
}

pub async fn forward_codex_sse<S, B>(
    mut byte_stream: S,
    tx: tokio::sync::mpsc::Sender<Result<ResponseStreamEvent, LlmError>>,
) where
    S: Stream<Item = Result<B, reqwest::Error>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut buffer = String::new();

    loop {
        let chunk = tokio::select! {
            _ = tx.closed() => return,
            chunk = byte_stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                let _ = tx.send(Err(LlmError::Http(e.to_string()))).await;
                return;
            }
        };
        buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()));

        while let Some(pos) = buffer.find("\n\n") {
            let frame = buffer[..pos].to_string();
            buffer.drain(..pos + 2);
            for event in parse_codex_sse_frame(&frame) {
                if tx.send(event).await.is_err() {
                    return;
                }
            }
        }
    }

    if !buffer.trim().is_empty() {
        for event in parse_codex_sse_frame(&buffer) {
            let _ = tx.send(event).await;
        }
        let _ = tx
            .send(Err(LlmError::Http(
                "Codex SSE stream closed mid-frame".into(),
            )))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn d41_dropping_receiver_cancels_stalled_response_body() {
        let stream = futures_util::stream::pending::<Result<Vec<u8>, reqwest::Error>>();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let forward = tokio::spawn(forward_codex_sse(stream, tx));

        drop(rx);

        tokio::time::timeout(std::time::Duration::from_millis(250), forward)
            .await
            .expect("stalled SSE body should be dropped when its receiver closes")
            .expect("forward task should exit cleanly");
    }
}
