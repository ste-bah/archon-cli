//! Weight persistence with CRC32 validation.

use std::io::{Read, Write};
use std::path::Path;

/// Magic bytes for weight file format.
const MAGIC: [u8; 4] = [0x47, 0x4E, 0x4E, 0x57]; // "GNNW"
/// Format version.
const VERSION: u32 = 1;

/// Error type for weight operations.
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
            WeightError::CrcMismatch { expected, actual } => {
                write!(
                    f,
                    "CRC32 mismatch: expected {:#010x}, got {:#010x}",
                    expected, actual
                )
            }
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

/// Weight file manager with CRC32 integrity validation.
pub struct WeightManager;

impl WeightManager {
    /// Save weights to a binary file with CRC32 checksum.
    ///
    /// Format: [MAGIC:4][VERSION:4][COUNT:4][WEIGHTS:count*4][CRC32:4]
    pub fn save(weights: &[f32], path: &Path) -> Result<(), WeightError> {
        let mut buf = Vec::new();

        // Magic
        buf.extend_from_slice(&MAGIC);
        // Version
        buf.extend_from_slice(&VERSION.to_le_bytes());
        // Weight count
        let count = weights.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());
        // Weight data
        for w in weights {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        // CRC32 of everything so far
        let crc = crc32(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        let mut file = std::fs::File::create(path)?;
        file.write_all(&buf)?;
        file.flush()?;
        Ok(())
    }

    /// Load weights from a binary file and validate CRC32.
    pub fn load(path: &Path) -> Result<Vec<f32>, WeightError> {
        let mut file = std::fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        // Minimum size: 4 (magic) + 4 (version) + 4 (count) + 4 (crc) = 16
        if buf.len() < 16 {
            return Err(WeightError::InvalidData("File too small".into()));
        }

        // Verify CRC32 (last 4 bytes)
        let data_len = buf.len() - 4;
        let stored_crc = u32::from_le_bytes([
            buf[data_len],
            buf[data_len + 1],
            buf[data_len + 2],
            buf[data_len + 3],
        ]);
        let computed_crc = crc32(&buf[..data_len]);
        if stored_crc != computed_crc {
            return Err(WeightError::CrcMismatch {
                expected: stored_crc,
                actual: computed_crc,
            });
        }

        // Parse header
        let magic = &buf[0..4];
        if magic != MAGIC {
            return Err(WeightError::InvalidMagic);
        }

        let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if version != VERSION {
            return Err(WeightError::VersionMismatch(version));
        }

        let count = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;

        let expected_len = 12 + count * 4;
        if data_len != expected_len {
            return Err(WeightError::InvalidData(format!(
                "Expected {} bytes of data, got {}",
                expected_len, data_len
            )));
        }

        // Parse weights
        let mut weights = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 12 + i * 4;
            let val = f32::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
            ]);
            weights.push(val);
        }

        Ok(weights)
    }
}

/// CRC32 (ISO 3309 / ITU-T V.42) — table-driven implementation.
fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for i in 0..256u32 {
            let mut crc = i;
            for _ in 0..8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB88320;
                } else {
                    crc >>= 1;
                }
            }
            table[i as usize] = crc;
        }
        table
    });

    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}
