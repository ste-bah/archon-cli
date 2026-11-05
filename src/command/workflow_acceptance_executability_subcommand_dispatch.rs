//! Evidence shared by the verdict and proactive warning for plugin dispatch.
use super::*;

/// An absent candidate of a tool known to dispatch commands. An observed
/// listing is evidence too. Both callers supply the listing they computed,
/// so this decision never depends on which caller filled a cache first.
pub(super) fn unavailable_candidate(
    tool: &str,
    name: &str,
    listed: &Listing,
    at: &Context,
) -> bool {
    if at.on_path(&format!("{tool}-{name}")) {
        return false;
    }
    if matches!(listed, Listing::Commands(_)) {
        return true;
    }
    let prefix = format!("{tool}-");
    [at.path.as_deref(), at.host_path.as_deref()]
        .into_iter()
        .flatten()
        .flat_map(|path| which::search_dirs(path, at.windows))
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .any(|entry| {
            entry.path().is_file()
                && at
                    .bare_name(&entry.file_name().to_string_lossy())
                    .starts_with(&prefix)
        })
}
