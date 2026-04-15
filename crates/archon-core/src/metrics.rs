/// Trait for receiving channel instrumentation events.
/// Implementors must be Send + Sync (required for Arc<dyn ChannelMetricSink>).
pub trait ChannelMetricSink: Send + Sync {
    /// Called when a message is successfully sent into the channel.
    fn record_sent(&self);
    /// Called when a batch of messages is drained from the channel.
    /// `batch_size` is the number of messages in the drain batch.
    fn record_drained(&self, batch_size: u64);
}