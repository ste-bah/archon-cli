use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use process_wrap::tokio::ChildWrapper;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;

const PIPE_READ_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Debug)]
pub(super) struct CapturedOutput {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
    pub(super) read_error: Option<String>,
}

pub(super) fn shared_output_budget(max_output_bytes: usize) -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(max_output_bytes))
}

pub(super) fn spawn_wrapped_child(
    command: tokio::process::Command,
) -> std::io::Result<Box<dyn ChildWrapper>> {
    use process_wrap::tokio::{CommandWrap, KillOnDrop};

    let mut wrapper = CommandWrap::from(command);
    wrapper.wrap(KillOnDrop);
    #[cfg(unix)]
    wrapper.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    wrapper.wrap(process_wrap::tokio::JobObject);
    wrapper.spawn()
}

pub(super) fn spawn_counted_pipe_capture<T>(
    pipe: Option<T>,
    budget: Arc<AtomicUsize>,
    byte_count: Arc<AtomicUsize>,
) -> JoinHandle<CapturedOutput>
where
    T: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move { capture_pipe(pipe, budget, byte_count).await })
}

async fn capture_pipe<T>(
    pipe: Option<T>,
    budget: Arc<AtomicUsize>,
    byte_count: Arc<AtomicUsize>,
) -> CapturedOutput
where
    T: AsyncRead + Unpin,
{
    let mut output = CapturedOutput {
        bytes: Vec::new(),
        truncated: false,
        read_error: None,
    };
    let Some(mut pipe) = pipe else {
        return output;
    };
    let mut chunk = [0_u8; PIPE_READ_CHUNK_BYTES];
    loop {
        let read = match pipe.read(&mut chunk).await {
            Ok(0) => return output,
            Ok(read) => read,
            Err(error) => {
                output.read_error = Some(error.to_string());
                return output;
            }
        };
        byte_count.fetch_add(read, Ordering::Relaxed);
        let retained = reserve_output_bytes(&budget, read);
        output.bytes.extend_from_slice(&chunk[..retained]);
        output.truncated |= retained < read;
    }
}

fn reserve_output_bytes(budget: &AtomicUsize, requested: usize) -> usize {
    let mut available = budget.load(Ordering::Relaxed);
    loop {
        let retained = available.min(requested);
        match budget.compare_exchange_weak(
            available,
            available - retained,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return retained,
            Err(current) => available = current,
        }
    }
}

pub(super) fn bounded_command_output(
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    exit_code: i32,
    max_output_bytes: usize,
) -> String {
    let content = String::from_utf8_lossy(&[stdout.bytes, stderr.bytes].concat()).into_owned();
    let prefix = (exit_code != 0).then(|| format!("Exit code {exit_code}\n"));
    bounded_content(
        content,
        max_output_bytes,
        stdout.truncated || stderr.truncated,
        prefix.as_deref(),
    )
}

pub(super) fn bounded_text(content: String, max_bytes: usize) -> String {
    bounded_body(content, max_bytes, false)
}

fn bounded_content(
    content: String,
    max_bytes: usize,
    truncated: bool,
    prefix: Option<&str>,
) -> String {
    let prefix = prefix.unwrap_or("");
    if max_bytes == 0 {
        return String::new();
    }
    if prefix.len() >= max_bytes {
        return truncate_utf8(prefix, max_bytes);
    }
    let body = bounded_body(content, max_bytes - prefix.len(), truncated);
    format!("{prefix}{body}")
}

fn bounded_body(content: String, max_bytes: usize, truncated: bool) -> String {
    if !truncated && content.len() <= max_bytes {
        return content;
    }
    let marker = truncation_marker(max_bytes);
    let end = utf8_boundary(&content, max_bytes.saturating_sub(marker.len()));
    format!("{}{marker}", &content[..end])
}

fn truncation_marker(max_bytes: usize) -> String {
    let marker = format!("\n\nOutput truncated at {max_bytes} bytes");
    if marker.len() <= max_bytes {
        marker
    } else {
        ".".repeat(max_bytes.min(3))
    }
}

fn truncate_utf8(content: &str, max_bytes: usize) -> String {
    content[..utf8_boundary(content, max_bytes)].to_string()
}

fn utf8_boundary(content: &str, limit: usize) -> usize {
    let mut end = limit.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    end
}
