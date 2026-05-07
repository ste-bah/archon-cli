/// Errors from CozoDB-backed weight operations.
#[derive(Debug)]
pub enum WeightStoreError {
    /// No weights have been saved yet.
    NoVersions,
    /// Requested version not found.
    VersionNotFound(i64),
    /// CozoDB query or transaction failed.
    Db(String),
    /// Weight data was corrupted or could not be deserialized.
    Corrupted(String),
    /// The latest version has NaN weights; rollback required.
    NanWeights(String),
}

impl std::fmt::Display for WeightStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeightStoreError::NoVersions => write!(f, "No weight versions saved"),
            WeightStoreError::VersionNotFound(v) => write!(f, "Version {} not found", v),
            WeightStoreError::Db(msg) => write!(f, "Database error: {}", msg),
            WeightStoreError::Corrupted(msg) => write!(f, "Corrupted weight data: {}", msg),
            WeightStoreError::NanWeights(msg) => write!(f, "NaN weights detected: {}", msg),
        }
    }
}

impl std::error::Error for WeightStoreError {}

#[derive(Debug)]
pub enum WeightError {
    Io(std::io::Error),
    InvalidMagic,
    VersionMismatch(u32),
    CrcMismatch { expected: u32, actual: u32 },
    InvalidData(String),
}

impl std::fmt::Display for WeightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeightError::Io(e) => write!(f, "IO error: {}", e),
            WeightError::InvalidMagic => write!(f, "Invalid magic bytes in weight file"),
            WeightError::VersionMismatch(v) => write!(f, "Version mismatch: got {}", v),
            WeightError::CrcMismatch { expected, actual } => write!(
                f,
                "CRC32 mismatch: expected {:#010x}, got {:#010x}",
                expected, actual
            ),
            WeightError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
        }
    }
}

impl std::error::Error for WeightError {}

impl From<std::io::Error> for WeightError {
    fn from(e: std::io::Error) -> Self {
        WeightError::Io(e)
    }
}
