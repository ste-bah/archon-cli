//! Repository test material never lands as project data (Batch K, I1).
//!
//! Live, a remediation asked to make "the strategy spec references only
//! registered datasets" pass ran the product's real ingest command with a
//! repository TEST FIXTURE as its source (an 8-bar CSV under
//! `crates/<crate>/tests/fixtures/`). The audited project-input landing put
//! the ingested dataset and a registry entry for it into the live project,
//! and the check went green on data the fixture's own notes call "never a
//! coverage-grade dataset". Nothing in the landing asked where the data came
//! from.
//!
//! Every host path that lands a remediation's bytes into the project root
//! now asks, per landed file, before anything is written:
//!
//! - **Identity**: are these bytes a byte-identical copy of a file the
//!   repository tracks at a test or fixture location? (`git ls-tree` at the
//!   repository's HEAD, plus test-location files its working tree holds that
//!   are not yet committed -- the landing's own, or its wave's.)
//! - **Named source**: does this text (JSON, CSV, notes, ...) name such a
//!   tracked file by its repository path -- a `request.json` saying
//!   `"fixture": "crates/x/tests/fixtures/y.csv"`?
//!
//! A hit refuses the landing (every file of it, as any refusal does), keeps
//! the refused bytes as evidence under the run
//! (`write-coordination/project-inputs-refused/<stage>/<item>/`), and becomes
//! a HIGH finding for the unit whose text starts [`FIXTURE_FINDING_PREFIX`].
//!
//! # The heuristic, and its limits
//!
//! A test location is any path with a directory segment (compared
//! case-blind) in [`TEST_SEGMENTS`] -- `tests/`, `test/`, `fixtures/`,
//! `__fixtures__/`, `testdata/`, `spec/fixtures/` (by its `fixtures`
//! segment) and their kin -- or a file name containing `fixture`. It is a
//! naming convention, so:
//!
//! - test material kept anywhere else (a `samples/` directory, a fixture
//!   inlined in source) is not recognised;
//! - identity is exact: a fixture re-serialised, re-ordered, trimmed or
//!   edited by one byte is not a copy. A file under [`MIN_IDENTICAL_BYTES`]
//!   is never judged a copy (an empty file, `{}` or a CSV header collide by
//!   accident);
//! - a named source must be the full repository-relative path (or a path
//!   ending in it, such as an absolute path into the checkout); a bare file
//!   name is not a reference. Only non-code files are referable: a report
//!   citing a test SOURCE file (`tests/x.rs`) names a test, not a data
//!   source, and is not a hit;
//! - a tracked file under the acceptance policy's project inputs is the
//!   project's data, never test material, whatever its path says (a
//!   `data/test/` split);
//! - a repository git cannot read (no `.git`, no HEAD) judges nothing: the
//!   landing proceeds as before, and the reason is logged;
//! - it cannot tell a fixture the task spec explicitly makes the deliverable
//!   from one smuggled in: such a landing is refused too, and the finding
//!   says so for a person to overrule.
//!
//! The verifier's own judgement of every landing (`verification::
//! project_data_landings`) is the second line behind this one.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use crate::write_coordinator::worktree_isolation::run_git;

/// Every finding this module raises starts with this text.
pub const FIXTURE_FINDING_PREFIX: &str = "repository test fixture landed as project data";

/// Directory segments that mark a test location, compared lower-cased.
pub const TEST_SEGMENTS: &[&str] = &[
    "test",
    "tests",
    "__tests__",
    "testing",
    "testdata",
    "test_data",
    "test-data",
    "testfixtures",
    "fixture",
    "fixtures",
    "__fixtures__",
    "__snapshots__",
    "__mocks__",
    "golden",
    "goldens",
];

/// Below this many bytes a match is never judged a copy.
pub const MIN_IDENTICAL_BYTES: usize = 64;

/// Largest landed text searched for a named source.
const MAX_SCANNED_TEXT: usize = 8 << 20;

/// Source-code extensions: a test file with one is a test, not a data
/// source, so naming it is not a hit (its bytes still are, when copied).
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "go", "java", "kt", "kts", "c", "h", "cc",
    "cpp", "hpp", "cs", "rb", "php", "swift", "scala", "sh", "bash", "zsh", "ps1", "lua", "pl",
    "ex", "exs", "erl", "hs", "ml", "clj", "dart",
];

/// Whether `path` (repository-relative) is at a test or fixture location.
pub fn is_test_location(path: &str) -> bool {
    let parts: Vec<String> = path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let Some((name, dirs)) = parts.split_last() else {
        return false;
    };
    dirs.iter().any(|dir| TEST_SEGMENTS.contains(&dir.as_str())) || name.contains("fixture")
}

fn is_code(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| CODE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// One landed file judged to be repository test material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureHit {
    /// The landed path, relative to the project root.
    pub landed: String,
    /// The tracked test file, relative to the repository root.
    pub fixture: String,
    /// `true`: a byte-identical copy; `false`: its text names the fixture.
    pub identical: bool,
}

impl FixtureHit {
    /// The HIGH finding's text.
    pub fn finding(&self) -> String {
        let how = if self.identical {
            "a byte-identical copy of that tracked test file"
        } else {
            "the landed file names that tracked test file as its source"
        };
        format!(
            "{FIXTURE_FINDING_PREFIX}: {} from {} ({how}); project data must come from the \
             product's own real ingestion paths, never from repository test fixtures",
            self.landed, self.fixture
        )
    }
}

enum Candidate {
    Blob(String),
    File(PathBuf),
}

/// Largest uncommitted working-tree file read as a candidate.
const MAX_PENDING_BYTES: u64 = 64 << 20;

/// The repository's test material, read once per landing.
pub struct FixtureIndex {
    repo: PathBuf,
    by_size: BTreeMap<u64, Vec<(String, Candidate)>>,
    referable: HashSet<String>,
    blobs: RefCell<BTreeMap<String, Option<Vec<u8>>>>,
}

impl FixtureIndex {
    /// Every file `repo` tracks at HEAD at a test location, plus every
    /// test-location file its working tree holds that HEAD does not (a
    /// landing's own, or an earlier landing's of the same wave, not yet
    /// committed) -- except paths under `project_inputs`, which are the
    /// project's data, not test material.
    pub fn load(repo: &Path, project_inputs: &[PathBuf]) -> Self {
        let mut index = Self {
            repo: repo.to_path_buf(),
            by_size: BTreeMap::new(),
            referable: HashSet::new(),
            blobs: RefCell::new(BTreeMap::new()),
        };
        if !repo.join(".git").exists() {
            eprintln!(
                "fixture provenance: {} is not a git checkout; landed data is not judged",
                repo.display()
            );
            return index;
        }
        let data = |path: &str| {
            project_inputs
                .iter()
                .any(|input| Path::new(path).starts_with(input))
        };
        match run_git(&["ls-tree", "-r", "-l", "-z", "HEAD"], repo) {
            Ok(output) => {
                for entry in output.stdout.split(|byte| *byte == 0) {
                    index.add_tree_entry(&String::from_utf8_lossy(entry), &data);
                }
            }
            Err(error) => eprintln!(
                "fixture provenance: the tracked files of {} are unreadable ({error}); landed data is not judged",
                repo.display()
            ),
        }
        let changed = [
            &["diff", "--name-only", "-z", "HEAD"][..],
            &["ls-files", "--others", "--exclude-standard", "-z"][..],
        ];
        for args in changed {
            let Ok(output) = run_git(args, repo) else {
                continue;
            };
            for rel in output.stdout.split(|byte| *byte == 0) {
                let rel = String::from_utf8_lossy(rel).into_owned();
                if rel.is_empty() || !is_test_location(&rel) || data(&rel) {
                    continue;
                }
                let path = repo.join(&rel);
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_file() && meta.len() <= MAX_PENDING_BYTES {
                    index.add(&rel, Candidate::File(path), meta.len());
                }
            }
        }
        index
    }

    /// `<mode> blob <oid> <size>\t<path>`.
    fn add_tree_entry(&mut self, entry: &str, data: &dyn Fn(&str) -> bool) {
        let Some((meta, path)) = entry.split_once('\t') else {
            return;
        };
        let fields: Vec<&str> = meta.split_whitespace().collect();
        let [_, "blob", oid, size] = fields.as_slice() else {
            return;
        };
        let Ok(size) = size.parse::<u64>() else {
            return;
        };
        if is_test_location(path) && !data(path) {
            self.add(path, Candidate::Blob((*oid).to_string()), size);
        }
    }

    fn add(&mut self, path: &str, candidate: Candidate, size: u64) {
        if !is_code(path) {
            self.referable.insert(path.to_string());
        }
        let entries = self.by_size.entry(size).or_default();
        if !entries.iter().any(|(held, _)| held == path) {
            entries.push((path.to_string(), candidate));
        }
    }

    /// Whether any test material was found at all.
    pub fn is_empty(&self) -> bool {
        self.by_size.is_empty()
    }

    fn blob(&self, oid: &str) -> Option<Vec<u8>> {
        self.blobs
            .borrow_mut()
            .entry(oid.to_string())
            .or_insert_with(|| {
                run_git(&["cat-file", "blob", oid], &self.repo)
                    .ok()
                    .map(|output| output.stdout)
            })
            .clone()
    }

    /// Every way `bytes`, landing at `landed`, is repository test material.
    pub fn judge(&self, landed: &str, bytes: &[u8]) -> Vec<FixtureHit> {
        let mut hits = Vec::new();
        if bytes.len() >= MIN_IDENTICAL_BYTES
            && let Some(candidates) = self.by_size.get(&(bytes.len() as u64))
        {
            for (path, candidate) in candidates {
                let same = match candidate {
                    Candidate::File(file) => {
                        crate::write_coordinator::project_inputs::read_no_follow(file)
                            .is_ok_and(|held| held == bytes)
                    }
                    Candidate::Blob(oid) => self.blob(oid).is_some_and(|held| held == bytes),
                };
                if same {
                    hits.push(FixtureHit {
                        landed: landed.to_string(),
                        fixture: path.clone(),
                        identical: true,
                    });
                    break;
                }
            }
        }
        for fixture in self.named_sources(bytes) {
            if !hits.iter().any(|hit| hit.fixture == fixture) {
                hits.push(FixtureHit {
                    landed: landed.to_string(),
                    fixture,
                    identical: false,
                });
            }
        }
        hits
    }

    /// [`Self::judge`] for the file at `path`, read (never through a link)
    /// only when its size could be a copy or it is small enough to search;
    /// with its bytes when it is a hit, for the evidence.
    pub fn judge_file(&self, landed: &str, path: &Path) -> (Vec<FixtureHit>, Option<Vec<u8>>) {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return (Vec::new(), None);
        };
        // A link into the repository's test material is that material.
        if meta.file_type().is_symlink() {
            return (
                self.linked_fixture(landed, path).into_iter().collect(),
                None,
            );
        }
        let size = meta.len();
        let worth =
            meta.is_file() && (size <= MAX_SCANNED_TEXT as u64 || self.by_size.contains_key(&size));
        let Some(bytes) = worth
            .then(|| crate::write_coordinator::project_inputs::read_no_follow(path).ok())
            .flatten()
        else {
            return (Vec::new(), None);
        };
        let hits = self.judge(landed, &bytes);
        let kept = (!hits.is_empty()).then_some(bytes);
        (hits, kept)
    }

    /// The test-location file of the repository a link at `path` resolves
    /// to, if any (the link itself is never followed for reading).
    fn linked_fixture(&self, landed: &str, path: &Path) -> Option<FixtureHit> {
        let target = std::fs::read_link(path).ok()?;
        let target = path
            .parent()?
            .join(target)
            .canonicalize()
            .map(archon_shell::paths::plain)
            .ok()?;
        let repo = self
            .repo
            .canonicalize()
            .map(archon_shell::paths::plain)
            .ok()?;
        let rel = target
            .strip_prefix(&repo)
            .ok()?
            .to_string_lossy()
            .into_owned();
        is_test_location(&rel).then(|| FixtureHit {
            landed: landed.to_string(),
            fixture: rel,
            identical: true,
        })
    }

    /// The referable test files `bytes` names by repository path.
    fn named_sources(&self, bytes: &[u8]) -> BTreeSet<String> {
        let mut named = BTreeSet::new();
        if self.referable.is_empty() || bytes.len() > MAX_SCANNED_TEXT {
            return named;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return named;
        };
        let path_char =
            |c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '@' | '+');
        for token in text.split(|c: char| !path_char(c)) {
            if !token.contains('/') {
                continue;
            }
            let token = token.trim_start_matches("./");
            if self.referable.contains(token) {
                named.insert(token.to_string());
                continue;
            }
            for (at, _) in token.match_indices('/') {
                let suffix = &token[at + 1..];
                if self.referable.contains(suffix) {
                    named.insert(suffix.to_string());
                    break;
                }
            }
        }
        named
    }
}

/// Judge one landing's `files` (project-relative path, where its bytes are)
/// against `index`. On any hit the hit files' bytes and the findings are
/// kept as evidence for (`stage_id`, `item_id`), the findings are appended
/// to `findings`, and the refusal reason is returned.
pub fn refuse_test_material(
    run_root: &Path,
    index: &FixtureIndex,
    (stage_id, item_id): (&str, &str),
    files: &[(String, PathBuf)],
    findings: &mut Vec<String>,
) -> Option<String> {
    if files.is_empty() || index.is_empty() {
        return None;
    }
    let mut hits = Vec::new();
    let mut kept = Vec::new();
    for (rel, path) in files {
        let (found, bytes) = index.judge_file(rel, path);
        if let Some(bytes) = bytes {
            kept.push((rel.clone(), bytes));
        }
        hits.extend(found);
    }
    if hits.is_empty() {
        return None;
    }
    keep_evidence(run_root, stage_id, item_id, &hits, &kept);
    let found: Vec<String> = hits.iter().map(FixtureHit::finding).collect();
    let reason = format!(
        "{}; the landing was refused and its bytes kept at {}",
        found.join("; "),
        evidence_dir(run_root, stage_id, item_id).display()
    );
    findings.extend(found);
    Some(reason)
}

/// [`refuse_test_material`] for one file already in memory.
pub fn refuse_bytes(
    run_root: &Path,
    index: &FixtureIndex,
    (stage_id, item_id): (&str, &str),
    rel: &str,
    bytes: &[u8],
    findings: &mut Vec<String>,
) -> Option<String> {
    let hits = index.judge(rel, bytes);
    if hits.is_empty() {
        return None;
    }
    keep_evidence(
        run_root,
        stage_id,
        item_id,
        &hits,
        &[(rel.to_string(), bytes.to_vec())],
    );
    let found: Vec<String> = hits.iter().map(FixtureHit::finding).collect();
    let reason = format!(
        "{}; the landing was refused and its bytes kept at {}",
        found.join("; "),
        evidence_dir(run_root, stage_id, item_id).display()
    );
    findings.extend(found);
    Some(reason)
}

/// The project inputs of the run at `run_root`, for [`FixtureIndex::load`].
pub fn project_inputs_of(run_root: &Path) -> Vec<PathBuf> {
    crate::write_coordinator::project_inputs::ProjectInputPolicy::for_run(run_root)
        .map(|policy| policy.inputs)
        .unwrap_or_default()
}

#[path = "fixture_provenance_evidence.rs"]
mod evidence;
pub use evidence::{evidence_dir, keep_evidence};

#[cfg(test)]
#[path = "fixture_provenance_tests.rs"]
mod tests;
