use super::{ApiError, classify_error};

#[test]
fn an_overload_preserves_a_provider_retry_after() {
    assert!(matches!(
        classify_error(529, "", Some("47")),
        ApiError::Overloaded {
            retry_after_secs: Some(47)
        }
    ));
    assert!(matches!(
        classify_error(529, "", None),
        ApiError::Overloaded {
            retry_after_secs: None
        }
    ));
}
