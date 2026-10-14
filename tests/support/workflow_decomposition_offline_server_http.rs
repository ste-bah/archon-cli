use std::io::Read;

pub(super) struct HttpRequest {
    pub(super) method: String,
    pub(super) path: String,
    version: String,
    headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl HttpRequest {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn keep_alive(&self) -> bool {
        match self.header("connection") {
            Some(value) if value.eq_ignore_ascii_case("close") => false,
            Some(value) if value.eq_ignore_ascii_case("keep-alive") => true,
            _ => self.version == "HTTP/1.1",
        }
    }
}

pub(super) fn next_request<R: Read>(
    reader: &mut R,
    pending: &mut Vec<u8>,
) -> std::io::Result<Option<HttpRequest>> {
    let head_end = loop {
        if let Some(index) = pending.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if pending.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP request headers exceed 64 KiB",
            ));
        }
        let mut chunk = [0u8; 8192];
        match reader.read(&mut chunk)? {
            0 if pending.is_empty() => return Ok(None),
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "incomplete HTTP request headers",
                ));
            }
            count => pending.extend_from_slice(&chunk[..count]),
        }
    };

    let head = String::from_utf8_lossy(&pending[..head_end]);
    let mut lines = head[..head.len().saturating_sub(4)].split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let version = request_line.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect::<Vec<_>>();
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse::<usize>())
        .transpose()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
        .unwrap_or_default();
    if length > 32 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP request body exceeds 32 MiB",
        ));
    }
    let request_end = head_end + length;
    while pending.len() < request_end {
        let mut chunk = [0u8; 8192];
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "incomplete HTTP request body",
            ));
        }
        pending.extend_from_slice(&chunk[..count]);
    }
    let body = pending[head_end..request_end].to_vec();
    pending.drain(..request_end);
    Ok(Some(HttpRequest {
        method,
        path,
        version,
        headers,
        body,
    }))
}
