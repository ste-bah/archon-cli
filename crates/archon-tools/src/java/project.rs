//! Which build tool a Java project uses, and what to invoke to drive it.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSystem {
    Gradle,
    Maven,
}

impl BuildSystem {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildSystem::Gradle => "gradle",
            BuildSystem::Maven => "maven",
        }
    }
}

/// How the build tool is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launcher {
    /// The project's own wrapper script. Always preferred when present: it
    /// pins the build-tool version the project was written against, so using
    /// it is the difference between reproducing the project's build and
    /// running a different one.
    Wrapper(PathBuf),
    /// A build tool found on PATH, used only when the project ships no wrapper.
    OnPath(&'static str),
}

impl Launcher {
    pub fn display(&self) -> String {
        match self {
            Launcher::Wrapper(path) => path.display().to_string(),
            Launcher::OnPath(name) => (*name).to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaProject {
    pub root: PathBuf,
    pub build_system: BuildSystem,
    pub launcher: Launcher,
}

/// Build files that identify a Gradle project. Checked before `pom.xml`.
const GRADLE_MARKERS: &[&str] = &[
    "settings.gradle",
    "settings.gradle.kts",
    "build.gradle",
    "build.gradle.kts",
];

/// Identify the project rooted at `root`.
///
/// Gradle is tested first because a Gradle build that also carries a `pom.xml`
/// (for publishing metadata, or as a leftover from a migration) is far more
/// common than a Maven build carrying a `build.gradle`. Returns `None` when the
/// directory holds neither, which is a real answer — not every directory handed
/// to this is a Java project.
pub fn detect(root: &Path) -> Option<JavaProject> {
    if GRADLE_MARKERS.iter().any(|m| root.join(m).is_file()) {
        return Some(JavaProject {
            root: root.to_path_buf(),
            build_system: BuildSystem::Gradle,
            launcher: gradle_launcher(root),
        });
    }
    if root.join("pom.xml").is_file() {
        return Some(JavaProject {
            root: root.to_path_buf(),
            build_system: BuildSystem::Maven,
            launcher: maven_launcher(root),
        });
    }
    None
}

fn gradle_launcher(root: &Path) -> Launcher {
    let wrapper = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    let path = root.join(wrapper);
    if path.is_file() {
        return Launcher::Wrapper(path);
    }
    Launcher::OnPath(if cfg!(windows) {
        "gradle.bat"
    } else {
        "gradle"
    })
}

fn maven_launcher(root: &Path) -> Launcher {
    let wrapper = if cfg!(windows) { "mvnw.cmd" } else { "mvnw" };
    let path = root.join(wrapper);
    if path.is_file() {
        return Launcher::Wrapper(path);
    }
    Launcher::OnPath(if cfg!(windows) { "mvn.cmd" } else { "mvn" })
}

/// The stage a run is driving. Kept separate from the build-tool arguments so
/// the ordering below reads as the loop it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Nothing else can run on code that does not build, so this goes first and
    /// is re-run after every change.
    Compile,
    /// Checkstyle, PMD, SpotBugs. SpotBugs works on bytecode, so this depends
    /// on Compile having succeeded.
    Analyze,
    Test,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Compile => "compile",
            Stage::Analyze => "analyze",
            Stage::Test => "test",
        }
    }
}

/// Arguments for one stage.
///
/// `--offline` is deliberately absent: both tools resolve their analysis
/// plugins from the network on first run, and forcing offline mode turns a
/// slow first build into a confusing failure.
pub fn stage_args(build_system: BuildSystem, stage: Stage) -> Vec<&'static str> {
    match (build_system, stage) {
        // `--console=plain` keeps Gradle from emitting the ANSI progress bar
        // into a captured pipe. Report files are the source of truth either
        // way, but unreadable console output helps nobody diagnose a failure.
        (BuildSystem::Gradle, Stage::Compile) => vec!["--console=plain", "classes", "testClasses"],
        (BuildSystem::Gradle, Stage::Analyze) => vec![
            "--console=plain",
            // Keep going after the first violating task: a run that stops at
            // Checkstyle never reaches SpotBugs, and the point of the pass is
            // to collect everything the tools can see in one go.
            "--continue",
            "checkstyleMain",
            "pmdMain",
            "spotbugsMain",
        ],
        (BuildSystem::Gradle, Stage::Test) => vec!["--console=plain", "--continue", "test"],
        (BuildSystem::Maven, Stage::Compile) => vec!["-B", "test-compile"],
        (BuildSystem::Maven, Stage::Analyze) => vec![
            "-B",
            // `check` goals fail the build on violation; the `:check` form
            // still writes the report first, which is what gets read.
            "checkstyle:checkstyle",
            "pmd:pmd",
            "spotbugs:spotbugs",
        ],
        (BuildSystem::Maven, Stage::Test) => vec!["-B", "-Dmaven.test.failure.ignore=true", "test"],
    }
}
