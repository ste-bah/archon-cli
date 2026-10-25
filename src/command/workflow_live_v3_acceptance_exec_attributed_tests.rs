#[tokio::test]
async fn attributed_teardown_keeps_the_stage_observation_usable() {
    let (_temp, _clear, result) =
        crate::command::acceptance_scratch_attributed_tests::attributed_observation().await;
    assert!(result.attributed_teardown.is_some(), "{result:?}");
    crate::command::acceptance_scratch_attributed_tests::assert_owner_finding(&result);
    super::ensure_observation_usable(&result).unwrap();
}

#[tokio::test]
async fn unattributed_teardown_still_fails_the_stage_observation() {
    let (_temp, _clear, result) =
        crate::command::acceptance_scratch_attributed_tests::unattributed_observation().await;
    assert!(result.attributed_teardown.is_none(), "{result:?}");
    assert!(!result.operational_errors.is_empty(), "{result:?}");
    assert!(
        super::ensure_observation_usable(&result).is_err(),
        "{result:?}"
    );
}
