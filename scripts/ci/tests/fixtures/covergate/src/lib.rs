//! Trivial fixture crate for exercising `cargo llvm-cov --fail-under-lines`.
//!
//! `add` is exercised by the unit test below so threshold 0 always passes.
//! `untested` is intentionally uncovered so a 100% line-coverage threshold
//! is guaranteed to fail, giving the self-test a reliable negative case.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn untested(x: i32) -> i32 {
    if x > 0 {
        x * 2
    } else {
        x - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_works() {
        assert_eq!(add(1, 2), 3);
    }
}
