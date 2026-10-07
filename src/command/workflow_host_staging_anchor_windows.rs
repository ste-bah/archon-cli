//! The staging anchor where std has no handle-relative API (see the parent
//! module's Windows limitation): every step first checks with
//! `symlink_metadata` that `host-command-staging` and the call directory are
//! real directories, refuses a link, and acts on paths.
use super::{STAGING_DIR, StagingAnchor};
use std::io;
use std::path::Path;

fn is_link(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Removes a link or file without following it.
fn remove_link(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        // A directory link is removed as a directory, its target untouched.
        Err(_) => std::fs::remove_dir(path),
        ok => ok,
    }
}

impl StagingAnchor {
    pub(super) fn open(run_root: &Path, name: &str) -> io::Result<Self> {
        let staging = run_root.join(STAGING_DIR);
        if let Ok(metadata) = std::fs::symlink_metadata(&staging)
            && (is_link(&metadata) || !metadata.is_dir())
        {
            remove_link(&staging)?;
        }
        std::fs::create_dir_all(&staging)?;
        Ok(Self {
            root: staging.join(name),
            name: name.into(),
        })
    }

    pub(super) fn make_call_dir(&self) -> io::Result<()> {
        self.check_parent()?;
        let parent = self.root.parent().expect("staging root has a parent");
        std::fs::create_dir(parent.join(&self.name))
    }

    fn check_parent(&self) -> io::Result<()> {
        let parent = self.root.parent().expect("staging root has a parent");
        let metadata = std::fs::symlink_metadata(parent)?;
        if is_link(&metadata) || !metadata.is_dir() {
            return Err(io::Error::other(format!(
                "'{}' was replaced by a link or file after the host created it",
                parent.display()
            )));
        }
        Ok(())
    }

    /// The call directory exists as a real directory (`false`: it is gone).
    fn check_call_dir(&self) -> io::Result<bool> {
        self.check_parent()?;
        match std::fs::symlink_metadata(&self.root) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_dir() => Err(self.replaced()),
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        self.check_call_dir()?
            .then_some(())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "staging is gone"))
    }

    pub(crate) fn remove_tree(&self) -> io::Result<()> {
        fn make_writable(path: &Path) -> io::Result<()> {
            let metadata = std::fs::symlink_metadata(path)?;
            if is_link(&metadata) {
                return Ok(());
            }
            let mut permissions = metadata.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(path, permissions)?;
            if metadata.is_dir() {
                for entry in std::fs::read_dir(path)? {
                    make_writable(&entry?.path())?;
                }
            }
            Ok(())
        }
        self.check_parent()?;
        match std::fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
            Ok(metadata) if is_link(&metadata) || !metadata.is_dir() => remove_link(&self.root),
            Ok(_) => {
                make_writable(&self.root)?;
                std::fs::remove_dir_all(&self.root)
            }
        }
    }

    pub(crate) fn read_file(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        if !self.check_call_dir()? {
            return Ok(None);
        }
        let path = self.root.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => std::fs::read(&path).map(Some),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn write_file(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if !self.check_call_dir()? {
            return Err(io::Error::new(io::ErrorKind::NotFound, "staging is gone"));
        }
        let path = self.root.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {
                std::fs::remove_dir_all(&path)?
            }
            Ok(_) => remove_link(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = options.open(&path)?;
        io::Write::write_all(&mut file, bytes)?;
        file.sync_all()
    }

    pub(crate) fn owner_only(&self, _name: &str) -> io::Result<()> {
        self.check_call_dir().map(drop)
    }

    pub(crate) fn scan(
        &self,
        read: bool,
        visit: &mut dyn FnMut(&Path, Option<&[u8]>) -> bool,
    ) -> io::Result<bool> {
        fn walk(
            root: &Path,
            dir: &Path,
            read: bool,
            visit: &mut dyn FnMut(&Path, Option<&[u8]>) -> bool,
        ) -> io::Result<bool> {
            let mut removed = false;
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                let metadata = std::fs::symlink_metadata(&path)?;
                if metadata.is_dir() && !is_link(&metadata) {
                    removed |= walk(root, &path, read, visit)?;
                    continue;
                }
                if read && !metadata.is_file() {
                    continue;
                }
                let bytes = if read {
                    Some(std::fs::read(&path)?)
                } else {
                    None
                };
                let rel = path.strip_prefix(root).unwrap_or(&path);
                if visit(rel, bytes.as_deref()) {
                    remove_link(&path)?;
                    removed = true;
                }
            }
            Ok(removed)
        }
        if !self.check_call_dir()? {
            return Ok(false);
        }
        walk(&self.root, &self.root, read, visit)
    }
}
