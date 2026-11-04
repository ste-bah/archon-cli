use super::*;

pub(super) fn require_blake3(algorithm: &str, file: &str) -> SkeletonResult<()> {
    if algorithm == "blake3" {
        Ok(())
    } else {
        invalid(format!(
            "{file} algorithm must be 'blake3'; re-run `workflow freeze-skeleton`"
        ))
    }
}

pub(super) fn read_required(path: &Path) -> SkeletonResult<Vec<u8>> {
    std::fs::read(path).map_err(|read| {
        let mut failure = io_error(&read, format!(
            "required task-set artifact {} could not be read: {read}; restore it or run the named freeze command",
            path.display()
        ));
        failure.io_kind = Some(read.kind());
        failure
    })
}

pub(super) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> SkeletonResult<T> {
    let bytes = read_required(path)?;
    serde_json::from_slice(&bytes).map_err(|parse| {
        error(format!(
            "{} is malformed JSON: {parse}; repair it or re-run the named freeze command",
            path.display()
        ))
    })
}

pub(super) fn invalid<T>(message: impl Into<String>) -> SkeletonResult<T> {
    Err(error(message))
}

pub(super) fn io_error(source: &std::io::Error, message: String) -> TaskSkeletonError {
    let mut fault = error(message);
    fault.io_cause = Some((source.kind(), source.raw_os_error()));
    fault.io_kind = Some(source.kind());
    fault
}

pub(super) fn error(message: impl Into<String>) -> TaskSkeletonError {
    TaskSkeletonError {
        message: message.into(),
        defects: Vec::new(),
        io_cause: None,
        io_kind: None,
    }
}
