use std::path::PathBuf;

const DEFAULT_STORE_DIR: &str = "doc-vector-store";

pub fn default_store_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("ARCHON_DOC_VECTOR_STORE_DIR") {
        return PathBuf::from(path);
    }
    #[cfg(test)]
    {
        std::env::temp_dir()
            .join(format!("archon-{DEFAULT_STORE_DIR}-tests"))
            .join(format!(
                "test-{}-{}",
                std::process::id(),
                test_thread_suffix()
            ))
    }
    #[cfg(not(test))]
    {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".archon")
            .join(DEFAULT_STORE_DIR)
    }
}

pub(super) fn num_parallelism() -> i32 {
    std::thread::available_parallelism()
        .map(|count| count.get().min(8) as i32)
        .unwrap_or(2)
}

#[cfg(test)]
pub(super) fn test_thread_suffix() -> String {
    format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}
