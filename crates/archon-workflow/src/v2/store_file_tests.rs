use super::*;

type Hook = Box<dyn FnOnce(&Path)>;

thread_local! {
    /// Run once on this thread between the checks and the open: a test
    /// swaps the entry there, as a writer racing the reader would.
    static BEFORE_OPEN: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn before_open(path: &Path) {
    if let Some(hook) = BEFORE_OPEN.with(|hook| hook.borrow_mut().take()) {
        hook(path);
    }
}

#[cfg(unix)]
mod unix {
    use super::*;

    fn mkfifo(path: &Path) {
        let raw = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0, "{path:?}");
    }

    /// `read_store_file_in(path, root)` on its own thread, with `swap` run
    /// between its checks and its open: a read that blocks fails the test
    /// after a few seconds instead of hanging the suite.
    fn read_swapped(path: &Path, root: &Path, swap: Option<fn(&Path)>) -> std::io::Result<Vec<u8>> {
        let (path, root) = (path.to_path_buf(), root.to_path_buf());
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Some(swap) = swap {
                BEFORE_OPEN.with(|hook| *hook.borrow_mut() = Some(Box::new(swap)));
            }
            let _ = done.send(read_store_file_in(&path, &root));
        });
        wait.recv_timeout(std::time::Duration::from_secs(10))
            .expect("read_store_file blocked")
    }

    fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        read_swapped(path, own_dir(path), None)
    }

    fn refusal(result: std::io::Result<Vec<u8>>) -> Option<StoreRefusal> {
        store_file_refusal(&result.expect_err("refused"))
    }

    #[test]
    fn a_regular_file_and_a_link_to_one_in_its_store_directory_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("record.json");
        std::fs::write(&file, b"{}").unwrap();
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink("record.json", &link).unwrap();
        assert_eq!(read(&file).unwrap(), b"{}");
        assert_eq!(read(&link).unwrap(), b"{}");
        assert_eq!(classify_store_entry(&link, dir.path()).unwrap(), None);
        // From an archive, a link into the store directory above it is read
        // only when its reader names that directory.
        let archive = dir.path().join("archive");
        std::fs::create_dir(&archive).unwrap();
        let up = archive.join("up.json");
        std::os::unix::fs::symlink("../record.json", &up).unwrap();
        assert_eq!(refusal(read(&up)), Some(StoreRefusal::OutsideLink));
        assert_eq!(read_store_file_in(&up, dir.path()).unwrap(), b"{}");
    }

    #[test]
    fn every_other_entry_is_refused_unread() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.json");
        mkfifo(&fifo);
        let to_fifo = dir.path().join("to-fifo.json");
        std::os::unix::fs::symlink(&fifo, &to_fifo).unwrap();
        let target = outside.path().join("record.json");
        std::fs::write(&target, b"{}").unwrap();
        let out = dir.path().join("out.json");
        std::os::unix::fs::symlink(&target, &out).unwrap();
        let nested = dir.path().join("nested.json");
        std::fs::create_dir(&nested).unwrap();
        let big = dir.path().join("big.json");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_STORE_FILE_BYTES + 1)
            .unwrap();
        for (path, expected) in [
            (&fifo, StoreRefusal::Special),
            (&to_fifo, StoreRefusal::Special),
            (&out, StoreRefusal::OutsideLink),
            (&nested, StoreRefusal::Directory),
            (&big, StoreRefusal::Oversized),
        ] {
            assert_eq!(refusal(read(path)), Some(expected), "{path:?}");
            assert_eq!(
                classify_store_entry(path, dir.path()).unwrap(),
                Some(expected)
            );
        }
    }

    #[test]
    fn a_link_deeper_than_the_store_directory_or_a_linked_directory_is_refused() {
        let call = tempfile::tempdir().unwrap();
        let (archive, revoked) = (call.path().join("superseded"), call.path().join("revoked"));
        std::fs::create_dir(&archive).unwrap();
        std::fs::create_dir(&revoked).unwrap();
        std::fs::write(revoked.join("old.json"), b"{}").unwrap();
        let into_revoked = archive.join("revived.json");
        std::os::unix::fs::symlink("../revoked/old.json", &into_revoked).unwrap();
        let read = read_swapped(&into_revoked, call.path(), None);
        assert_eq!(refusal(read), Some(StoreRefusal::OutsideLink));
        // An archive directory that is a link: its regular files live
        // somewhere else.
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("record.json"), b"{}").unwrap();
        let linked = call.path().join("linked");
        std::os::unix::fs::symlink(elsewhere.path(), &linked).unwrap();
        let read = read_swapped(&linked.join("record.json"), call.path(), None);
        assert_eq!(refusal(read), Some(StoreRefusal::LinkedDirectory));
    }

    #[test]
    fn an_entry_swapped_after_its_check_is_refused_on_the_open_handle() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("record.json");
        std::fs::write(&file, b"{}").unwrap();
        // A FIFO in its place: opened without blocking, refused by fstat.
        let read = read_swapped(
            &file,
            dir.path(),
            Some(|path| {
                std::fs::remove_file(path).unwrap();
                mkfifo(path);
            }),
        );
        assert_eq!(refusal(read), Some(StoreRefusal::Special));
        // A link in its place: the open does not follow it.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, b"{}").unwrap();
        let read = read_swapped(
            &file,
            dir.path(),
            Some(|path| {
                let aside = path.with_extension("aside");
                std::fs::rename(path, &aside).unwrap();
                std::os::unix::fs::symlink(&aside, path).unwrap();
            }),
        );
        assert_eq!(refusal(read), Some(StoreRefusal::Changed));
    }

    #[test]
    fn a_file_at_the_bound_is_read_and_a_broken_link_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let at_bound = dir.path().join("at-bound.json");
        std::fs::File::create(&at_bound)
            .unwrap()
            .set_len(MAX_STORE_FILE_BYTES)
            .unwrap();
        assert_eq!(read(&at_bound).unwrap().len() as u64, MAX_STORE_FILE_BYTES);
        let broken = dir.path().join("broken.json");
        std::os::unix::fs::symlink("missing.json", &broken).unwrap();
        let error = read(&broken).expect_err("a broken link cannot be read");
        assert_eq!(store_file_refusal(&error), None);
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
}
