/// Serialize a weight matrix to bytes: [out_dim: u32 LE][in_dim: u32 LE][f32 LE * n]
pub(super) fn serialize_matrix(w: &[Vec<f32>]) -> Vec<u8> {
    let out_dim = w.len() as u32;
    let in_dim = w.first().map(|r| r.len()).unwrap_or(0) as u32;
    let mut buf = Vec::with_capacity(8 + out_dim as usize * in_dim as usize * 4);
    buf.extend_from_slice(&out_dim.to_le_bytes());
    buf.extend_from_slice(&in_dim.to_le_bytes());
    for row in w {
        for &v in row {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    buf
}

/// Deserialize a weight matrix from bytes.
pub(super) fn deserialize_matrix(data: &[u8]) -> Option<Vec<Vec<f32>>> {
    if data.len() < 8 {
        return None;
    }
    let out_dim = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let in_dim = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let expected = 8 + out_dim * in_dim * 4;
    if data.len() != expected {
        return None;
    }
    let mut w = Vec::with_capacity(out_dim);
    for i in 0..out_dim {
        let mut row = Vec::with_capacity(in_dim);
        for j in 0..in_dim {
            let offset = 8 + (i * in_dim + j) * 4;
            let val = f32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            row.push(val);
        }
        w.push(row);
    }
    Some(w)
}

/// Serialize a bias vector to bytes: [len: u32 LE][f32 LE * len]
pub(super) fn serialize_vector(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + v.len() * 4);
    buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
    for &x in v {
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf
}

/// Deserialize a bias vector from bytes.
pub(super) fn deserialize_vector(data: &[u8]) -> Option<Vec<f32>> {
    if data.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let expected = 4 + len * 4;
    if data.len() != expected {
        return None;
    }
    let mut v = Vec::with_capacity(len);
    for i in 0..len {
        let offset = 4 + i * 4;
        let val = f32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        v.push(val);
    }
    Some(v)
}
