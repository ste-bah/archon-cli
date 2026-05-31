#[derive(Clone, Debug, Default)]
pub struct IndexResult {
    pub indexed: usize,
    pub failed: usize,
    pub skipped: usize,
}
