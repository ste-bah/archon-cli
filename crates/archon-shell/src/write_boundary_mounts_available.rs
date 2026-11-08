use super::*;

impl MountSeals {
    pub(in crate::write_boundary::landlock) fn build_available(
        protected: &[PathBuf],
    ) -> Result<(Self, Vec<String>), String> {
        Self::available_with(protected, topology::mountpoints, |mounts| {
            mounts.preflight()
        })
    }

    fn available_with(
        protected: &[PathBuf],
        topology: impl FnOnce() -> Result<Vec<PathBuf>, String>,
        mut preflight: impl FnMut(&Self) -> Result<(), String>,
    ) -> Result<(Self, Vec<String>), String> {
        let mountpoints = topology().map_err(|reason| {
            format!(
                "{} not sealed: {reason}",
                protected
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
        let writable = [PathBuf::from(std::path::MAIN_SEPARATOR_STR)];
        let mut combined = Self::prepare(&[], &writable, &mountpoints)?;
        let mut notes = Vec::new();
        let mut prepared = Vec::new();
        for path in protected {
            match Self::prepare(std::slice::from_ref(path), &writable, &mountpoints) {
                Ok(mounts) => match combined.merge(&mounts) {
                    Ok(()) => prepared.push((path, mounts)),
                    Err(reason) => notes.push(format!("{} not sealed: {reason}", path.display())),
                },
                Err(reason) => notes.push(format!("{} not sealed: {reason}", path.display())),
            }
        }
        if !combined.seals.is_empty() && preflight(&combined).is_err() {
            combined.seals.clear();
            for (path, mounts) in prepared {
                match preflight(&mounts) {
                    Ok(()) => combined
                        .merge(&mounts)
                        .map_err(|reason| format!("{} not sealed: {reason}", path.display()))?,
                    Err(reason) => notes.push(format!("{} not sealed: {reason}", path.display())),
                }
            }
            // Individually successful sets may still fail when combined.
            if !combined.seals.is_empty() {
                preflight(&combined).map_err(|reason| {
                    format!(
                        "{} not sealed: {reason}",
                        protected
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                })?;
            }
        }
        Ok((combined, notes))
    }

    fn merge(&mut self, mounts: &Self) -> Result<(), String> {
        let additional = mounts
            .seals
            .iter()
            .filter(|seal| !self.seals.iter().any(|existing| existing.path == seal.path))
            .count();
        if self.seals.len() + additional > 256 {
            return Err("seal status capacity exceeded".into());
        }
        for seal in &mounts.seals {
            if let Some(existing) = self.seals.iter_mut().find(|s| s.path == seal.path) {
                existing.readonly |= seal.readonly;
            } else {
                self.seals.push(seal.clone());
            }
        }
        let planned: Vec<_> = self
            .seals
            .iter()
            .map(|seal| PathBuf::from(std::ffi::OsStr::from_bytes(seal.path.as_bytes())))
            .collect();
        for seal in &mut self.seals {
            let path = std::path::Path::new(std::ffi::OsStr::from_bytes(seal.path.as_bytes()));
            seal.submounts |= topology::contains_mount(path, &planned);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn test_available(
        protected: &[PathBuf],
        topology: impl FnOnce() -> Result<Vec<PathBuf>, String>,
        preflight: impl FnMut(&Self) -> Result<(), String>,
    ) -> Result<(Self, Vec<String>), String> {
        Self::available_with(protected, topology, preflight)
    }
}
