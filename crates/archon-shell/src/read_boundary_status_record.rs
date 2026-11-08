/// Fixed-size operation, error, entry index, and working-directory re-entry flag.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StatusRecord(pub(crate) [i32; 4]);

impl StatusRecord {
    pub(crate) fn encode(&self) -> [u8; 16] {
        let mut bytes = [0; 16];
        for (index, value) in self.0.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub(crate) fn decode(bytes: [u8; 16]) -> Self {
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
        Self(fields)
    }
}

#[cfg(test)]
#[test]
fn status_record_encode_decode() {
    for fields in [[0; 4], [8, 13, 7, 1], [i32::MAX, i32::MIN, -1, 0]] {
        let record = StatusRecord(fields);
        assert_eq!(StatusRecord::decode(record.encode()), record);
    }
    assert_eq!(
        StatusRecord([1, 2, 3, 4]).encode(),
        [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]
    );
}
