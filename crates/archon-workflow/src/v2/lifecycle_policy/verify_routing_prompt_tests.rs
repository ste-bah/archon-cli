//! Issue-85: only the verifier can decide not to withhold acceptance, so the
//! prose has to tell it when that is right.
//!
//! A verification said, in its own summary, that the task's implementation was
//! real and correct and every declared suite passed — and still returned a
//! non-accepted verdict, over a defect in a file no task in the universe
//! declares. No branch may write such a file, so remediation returns a no-op
//! and the cycle repeats to budget exhaustion. The host must not promote a
//! verdict from prose, and the lifecycle loop keys on the verdict rather than
//! on the gap set, which leaves the guidance as the only honest lever.

const PROMPTS: [&str; 4] = [
    crate::v2::lifecycle_prompts::VERIFICATION_WAVE_TASK,
    crate::v2::lifecycle_prompts::RETRY_VERIFICATION_WAVE_TASK,
    crate::v2::lifecycle_prompts::POST_REMEDIATION_VERIFICATION_WAVE_TASK,
    crate::v2::lifecycle_prompts::REVIEW_VERIFICATION_WAVE_TASK,
];

#[test]
fn verification_prompts_release_a_task_from_a_defect_no_task_declares() {
    for prompt in PROMPTS {
        for required in [
            // The rule, and why refusing cannot help.
            "is a residual gap, not a reason to withhold acceptance",
            "no branch can ever be dispatched to write a file no task owns",
            // The exemption is fail-closed by construction: a verifier that
            // cannot establish both facts from what it was given must not
            // use it. Which matters, because how much a verifier is told
            // about declared scope differs by dispatch path.
            "Never assume the two facts this rests on",
            "if you cannot establish both from what you were given, do not use this exemption",
            // The defect is recorded, never dropped.
            "Record a residual gap naming the file and stating that no task declares it",
            // What is judged instead.
            "judge this task on what it owns",
            // And the guard against using it as an escape hatch.
            "The exemption is narrow and never an escape hatch",
            "still fails the task exactly as before",
        ] {
            assert!(
                prompt.contains(required),
                "missing {required:?} in {prompt}"
            );
        }
    }
}

/// The exemption never softens the rules it sits beside: a red test the task
/// owns, and an unevidenced pre-existing claim, still fail exactly as before.
#[test]
fn the_exemption_does_not_displace_the_rules_it_sits_beside() {
    for prompt in PROMPTS {
        assert!(
            prompt.contains("never mark a failure pre-existing without that evidence"),
            "{prompt}"
        );
        assert!(
            prompt.contains("Never treat a zero-match as a pass on its own"),
            "{prompt}"
        );
        assert!(prompt.contains("Do not modify files."), "{prompt}");
    }
}
