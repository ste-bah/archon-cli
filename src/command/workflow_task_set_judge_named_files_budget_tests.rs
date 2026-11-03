use std::cell::RefCell;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

thread_local! {
    static BODY_READS: RefCell<Arc<AtomicUsize>> = RefCell::new(Arc::default());
}

pub(crate) fn record_body_read() {
    BODY_READS.with(|reads| reads.borrow().fetch_add(1, Ordering::Relaxed));
}

pub(crate) fn body_reads() -> usize {
    BODY_READS.with(|reads| reads.borrow().load(Ordering::Relaxed))
}

pub(crate) fn body_read_observer() -> Arc<AtomicUsize> {
    BODY_READS.with(|reads| Arc::clone(&reads.borrow()))
}

pub(crate) fn set_body_read_observer(reads: Arc<AtomicUsize>) {
    BODY_READS.with(|observer| *observer.borrow_mut() = reads);
}

#[test]
fn binary_content_is_retained_for_identity_without_spending_source_budget() {
    let bytes = b"\x7fELF\0binary payload".to_vec();
    let mut remaining = Some(1);
    let file = super::ResolvedNamedFile::supplied("input".into(), bytes.clone(), &mut remaining);
    assert_eq!(file.status, "not_judgeable");
    assert_eq!(file.bytes, Some(bytes));
    assert_eq!(remaining, Some(1));
}
