//! Capture names once; inspect executable candidates only when requested.
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

struct Catalog {
    directories: Vec<(PathBuf, BTreeMap<OsString, OsString>)>,
    extensions: Vec<OsString>,
    cache: Mutex<BTreeMap<OsString, Option<PathBuf>>>,
}
static CATALOG: OnceLock<Catalog> = OnceLock::new();

fn key(name: &OsStr) -> OsString {
    if cfg!(windows) {
        name.to_string_lossy().to_lowercase().into()
    } else {
        name.to_owned()
    }
}
fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| Catalog {
        directories: super::startup_path()
            .as_ref()
            .map_or_else(Vec::new, |path| {
                std::env::split_paths(path)
                    .map(|directory| {
                        let names = std::fs::read_dir(&directory)
                            .into_iter()
                            .flatten()
                            .flatten()
                            .map(|entry| {
                                let name = entry.file_name();
                                (key(&name), name)
                            })
                            .collect();
                        (directory, names)
                    })
                    .collect()
            }),
        extensions: if cfg!(windows) {
            std::env::var_os("PATHEXT")
                .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
                .to_string_lossy()
                .split(';')
                .filter(|s| !s.is_empty())
                .map(OsString::from)
                .collect()
        } else {
            Vec::new()
        },
        cache: Mutex::new(BTreeMap::new()),
    })
}
pub(super) fn initialize() {
    let _ = catalog();
}
impl Catalog {
    fn resolve(&self, name: &OsStr, mut inspect: impl FnMut(&Path) -> bool) -> Option<PathBuf> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let name_key = key(name);
        if let Some(result) = cache.get(&name_key) {
            return result.clone();
        }
        let mut candidates = vec![name_key.clone()];
        let mut seen = BTreeSet::new();
        for extension in &self.extensions {
            let mut candidate = name_key.clone();
            candidate.push(extension);
            let candidate = key(&candidate);
            if seen.insert(candidate.clone()) {
                candidates.push(candidate);
            }
        }
        let result = self.directories.iter().find_map(|(directory, names)| {
            candidates.iter().find_map(|candidate| {
                let path = directory.join(names.get(candidate)?);
                inspect(&path).then_some(path)
            })
        });
        cache.insert(name_key, result.clone());
        result
    }
}
#[cfg(not(windows))]
fn candidate(path: &Path) -> bool {
    // Resolution must validate this recorded name, never an appended sibling.
    which::which_in(
        path,
        None::<&OsStr>,
        Path::new(std::path::MAIN_SEPARATOR_STR),
    )
    .is_ok_and(|resolved| resolved == path)
}
#[cfg(windows)]
fn candidate(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let present = path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file() || metadata.file_type().is_symlink());
    if !present {
        return false;
    }
    if path.extension().is_some() {
        return true;
    }
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return false;
    }
    wide.push(0);
    let mut kind = 0;
    // SAFETY: terminated input and a writable output; validates only this name.
    unsafe {
        windows_sys::Win32::Storage::FileSystem::GetBinaryTypeW(wide.as_ptr(), &mut kind) != 0
    }
}
pub(super) fn resolve(name: &OsStr) -> Option<PathBuf> {
    catalog().resolve(name, candidate)
}
pub(super) fn unavailable(name: &OsStr) -> OsString {
    std::env::current_exe()
        .map(|path| {
            path.join(Path::new(name).file_name().unwrap_or(name))
                .into_os_string()
        })
        .unwrap_or_default()
}
pub(super) fn program(name: &OsStr) -> OsString {
    if Path::new(name).components().count() != 1 || Path::new(name).is_absolute() {
        return name.to_owned();
    }
    resolve(name)
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| unavailable(name))
}

#[cfg(test)]
#[path = "spawn_host_programs_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn recorded() -> Vec<(OsString, PathBuf)> {
    catalog()
        .directories
        .iter()
        .flat_map(|(_, names)| names.keys())
        .filter_map(|name| resolve(name).map(|path| (name.clone(), path)))
        .collect()
}
