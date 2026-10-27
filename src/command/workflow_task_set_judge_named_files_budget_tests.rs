use std::cell::Cell;

thread_local! {
    static BODY_READS: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn record_body_read() {
    BODY_READS.with(|reads| reads.set(reads.get() + 1));
}

pub(crate) fn body_reads() -> usize {
    BODY_READS.with(Cell::get)
}
