// Fixture: clean Rust source with no banned patterns.
// Used by tui-banned-patterns-gate.selftest.sh to assert the gate exits 0
// when a scan root contains only clean files.

fn main() {
    let _answer: u32 = 42;
}
