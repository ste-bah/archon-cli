use futures_util::{Stream, StreamExt};

use crate::streaming::{StreamError, StreamEvent, parse_sse_event};

pub(crate) fn spawn_anthropic_stream_reader<S, B, E>(
    stream: S,
) -> tokio::sync::mpsc::Receiver<StreamEvent>
where
    S: Stream<Item = Result<B, E>> + Send + Unpin + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { read_anthropic_stream(stream, tx).await });
    rx
}

async fn read_anthropic_stream<S, B, E>(mut stream: S, tx: tokio::sync::mpsc::Sender<StreamEvent>)
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut buffer = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = tx.closed() => return,
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        match chunk {
            Ok(chunk) => buffer.extend_from_slice(chunk.as_ref()),
            Err(error) => {
                send_stream_error(&tx, "network", error).await;
                return;
            }
        }
        while let Some(frame_end) = find_sse_frame_end(&buffer) {
            let frame = buffer.drain(..frame_end).collect::<Vec<_>>();
            match handle_sse_frame(&frame, &tx).await {
                FrameOutcome::Continue => {}
                FrameOutcome::MessageStop | FrameOutcome::Terminal => return,
            }
        }
    }
    if !buffer.is_empty() {
        match handle_sse_frame(&buffer, &tx).await {
            FrameOutcome::Continue => {}
            FrameOutcome::MessageStop | FrameOutcome::Terminal => return,
        }
    }
    send_stream_error(&tx, "protocol", "stream ended before message_stop").await;
}

fn find_sse_frame_end(buffer: &[u8]) -> Option<usize> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| position + 2);
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4);
    match (lf, crlf) {
        (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
        (Some(frame_end), None) | (None, Some(frame_end)) => Some(frame_end),
        (None, None) => None,
    }
}

enum FrameOutcome {
    Continue,
    MessageStop,
    Terminal,
}

async fn handle_sse_frame(
    frame: &[u8],
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> FrameOutcome {
    let Ok(frame) = std::str::from_utf8(frame) else {
        send_stream_error(tx, "parse_error", "SSE frame was not UTF-8").await;
        return FrameOutcome::Terminal;
    };
    let Some((event_type, data)) = parse_sse_frame(frame) else {
        return FrameOutcome::Continue;
    };
    match parse_sse_event(&event_type, &data) {
        Ok(StreamEvent::MessageStop) => {
            if tx.send(StreamEvent::MessageStop).await.is_err() {
                FrameOutcome::Terminal
            } else {
                FrameOutcome::MessageStop
            }
        }
        Ok(StreamEvent::Error {
            error_type,
            message,
        }) => {
            let _ = tx
                .send(StreamEvent::Error {
                    error_type,
                    message,
                })
                .await;
            FrameOutcome::Terminal
        }
        Ok(event) => {
            if tx.send(event).await.is_err() {
                FrameOutcome::Terminal
            } else {
                FrameOutcome::Continue
            }
        }
        Err(StreamError::UnknownEvent(_)) => FrameOutcome::Continue,
        Err(error) => {
            send_stream_error(tx, "parse_error", error).await;
            FrameOutcome::Terminal
        }
    }
}

fn parse_sse_frame(frame: &str) -> Option<(String, String)> {
    let mut event_type = None;
    let mut data = Vec::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim_start().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    event_type.map(|event_type| (event_type, data.join("\n")))
}

async fn send_stream_error(
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    error_type: &str,
    error: impl std::fmt::Display,
) {
    let _ = tx
        .send(StreamEvent::Error {
            error_type: error_type.into(),
            message: error.to_string(),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    type Chunk = Result<Vec<u8>, &'static str>;

    fn controlled_stream(
        receiver: tokio::sync::mpsc::Receiver<Chunk>,
    ) -> impl Stream<Item = Chunk> + Send + Unpin + 'static {
        futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|chunk| (chunk, receiver))
        })
        .boxed()
    }

    fn text_delta(text: &str) -> Chunk {
        Ok(format!(
            "event: content_block_delta\ndata: {{\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}\n\n"
        )
        .into_bytes())
    }

    #[tokio::test]
    async fn emits_delta_before_source_stream_completes() {
        let (source_tx, source_rx) = tokio::sync::mpsc::channel(2);
        let mut events = spawn_anthropic_stream_reader(controlled_stream(source_rx));

        source_tx.send(text_delta("first")).await.unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_millis(100), events.recv())
            .await
            .expect("delta must arrive while source sender is still open")
            .expect("stream event");
        assert!(matches!(event, StreamEvent::TextDelta { text, .. } if text == "first"));

        source_tx
            .send(Ok(br#"event: message_stop
data: {"type":"message_stop"}

"#
            .to_vec()))
            .await
            .unwrap();
        assert!(matches!(
            events.recv().await,
            Some(StreamEvent::MessageStop)
        ));
    }

    #[tokio::test]
    async fn source_error_after_delta_is_emitted_as_network_error() {
        let (source_tx, source_rx) = tokio::sync::mpsc::channel(2);
        let mut events = spawn_anthropic_stream_reader(controlled_stream(source_rx));

        source_tx.send(text_delta("partial")).await.unwrap();
        assert!(matches!(
            events.recv().await,
            Some(StreamEvent::TextDelta { text, .. }) if text == "partial"
        ));

        source_tx.send(Err("connection reset")).await.unwrap();
        assert!(matches!(
            events.recv().await,
            Some(StreamEvent::Error { error_type, message })
                if error_type == "network" && message == "connection reset"
        ));
    }
}
