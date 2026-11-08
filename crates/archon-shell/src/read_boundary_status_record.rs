/// Fixed operation, error, entry index, re-entry flag and unapplied-entry bitmap.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StatusRecord(pub(crate) [i32; 4], pub(crate) [u64; 4]);

impl StatusRecord {
    pub(crate) fn encode(&self) -> [u8; 48] {
        let mut bytes = [0; 48];
        for (index, value) in self.0.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (index, value) in self.1.iter().enumerate() {
            bytes[16 + index * 8..24 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub(crate) fn decode(bytes: [u8; 48]) -> Self {
        let mut fields = [0; 4];
        for (index, value) in fields.iter_mut().enumerate() {
            let offset = index * 4;
            *value = i32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
        }
        let mut failed = [0; 4];
        for (index, value) in failed.iter_mut().enumerate() {
            *value = u64::from_le_bytes(bytes[16 + index * 8..24 + index * 8].try_into().unwrap());
        }
        Self(fields, failed)
    }
}

#[cfg(test)]
#[test]
fn status_record_encode_decode() {
    for fields in [[0; 4], [8, 13, 7, 1], [i32::MAX, i32::MIN, -1, 0]] {
        let record = StatusRecord(fields, [1, 1 << 63, u64::MAX, 0]);
        assert_eq!(StatusRecord::decode(record.encode()), record);
    }
    assert_eq!(
        StatusRecord([1, 2, 3, 4], [0; 4]).encode()[..16],
        [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]
    );
}
