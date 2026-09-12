//! A validation repair chain has one identity even when call ids are reused.
tokio::task_local! { static GENERATION: String; }

pub fn current() -> Option<String> {
    GENERATION.try_with(Clone::clone).ok()
}

pub async fn scope<T>(work: impl std::future::Future<Output = T>) -> T {
    GENERATION
        .scope(uuid::Uuid::new_v4().to_string(), work)
        .await
}
