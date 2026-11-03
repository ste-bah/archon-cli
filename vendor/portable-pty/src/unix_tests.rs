use super::*;

#[test]
fn active_landlock_refuses_failed_operator_read_and_write_rules() {
    for reason in ["injected read failure", "injected write failure"] {
        let error = require_operator_boundary::<()>(Err(reason.into()), true)
            .expect_err("active Landlock must refuse a failed ruleset");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("It was NOT run"));
        assert_eq!(
            require_operator_boundary::<()>(Err(reason.into()), false).unwrap(),
            None
        );
    }
    assert_eq!(require_operator_boundary(Ok(7), true).unwrap(), Some(7));
}
