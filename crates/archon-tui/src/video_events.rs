#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoIngestProgressEvent {
    pub video_id: String,
    pub segment_count: u32,
    pub latest_text: String,
    pub status: String,
}
