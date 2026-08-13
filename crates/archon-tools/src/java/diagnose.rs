//! Explaining why an analyser produced nothing.
//!
//! Knowing that SpotBugs wrote no report is useful; knowing *why* is what lets
//! someone fix it. The one cause worth naming explicitly is version skew
//! between an analysis plugin and the JDK running the build, because it is
//! invisible in every other signal: the build exits zero, the other analysers
//! report normally, and the failure looks exactly like a clean scan.

/// A class file's major version is its JDK feature version plus 44 — Java 25
/// emits 69, Java 21 emits 65.
///
/// Deriving the JDK from the number means this never needs a table of releases
/// to stay correct, which matters because the failure recurs with every new
/// Java version: whoever's plugin is older than their JDK hits it next.
const CLASS_FILE_MAJOR_OFFSET: u32 = 44;

/// Why the analysis stage came back empty, if the build output says.
///
/// Returns `None` when nothing recognisable is in the output — an absent
/// explanation is not an explanation that nothing is wrong.
pub fn explain_missing_reports(output: &str) -> Option<String> {
    let jdk = unsupported_class_file_jdk(output)?;
    let mut explanation = String::new();
    explanation.push_str(&format!(
        "An analysis tool could not read Java {jdk} class files and aborted before \
         writing its report. Its bundled bytecode reader predates that release.\n\n"
    ));
    explanation.push_str(
        "This is version skew between the analysis plugin and the JDK running the \
         build, not a problem in the code under analysis — and it is why the stage \
         produced no findings rather than no issues.\n\n",
    );
    explanation.push_str(&format!(
        "Fix it by raising the plugin to a release newer than Java {jdk}:\n"
    ));
    explanation.push_str("  Maven:  com.github.spotbugs:spotbugs-maven-plugin\n");
    explanation.push_str("  Gradle: id(\"com.github.spotbugs\")\n");
    explanation.push_str(
        "Or run the build on an older JDK. Do not treat this stage as clean until \
         one of those is done.",
    );
    Some(explanation)
}

/// The JDK feature version named by an "unsupported class file" error.
///
/// Every JVM bytecode reader phrases this the same way because the message
/// comes from ASM, which all of these tools embed.
fn unsupported_class_file_jdk(output: &str) -> Option<u32> {
    const MARKER: &str = "Unsupported class file major version ";
    let start = output.find(MARKER)? + MARKER.len();
    let digits: String = output[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let major: u32 = digits.parse().ok()?;
    major.checked_sub(CLASS_FILE_MAJOR_OFFSET)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact text SpotBugs 4.8.6 emits on JDK 25, which is what motivated
    /// this: the build exited zero and the security analyser was dead.
    const SPOTBUGS_ON_JDK_25: &str = "\
     [java] The following errors occurred during analysis:
     [java]   Error scanning java/lang/Object for referenced classes
     [java]     java.lang.IllegalArgumentException: Unsupported class file major version 69
     [java]       At edu.umd.cs.findbugs.FindBugs2.buildReferencedClassSet(FindBugs2.java:806)
";

    #[test]
    fn the_jdk_is_derived_from_the_class_file_version() {
        assert_eq!(unsupported_class_file_jdk(SPOTBUGS_ON_JDK_25), Some(25));
    }

    /// Arithmetic rather than a lookup table, so a JDK released after this code
    /// was written still reports correctly.
    #[test]
    fn a_future_jdk_is_named_without_a_release_table() {
        let output = "Unsupported class file major version 71";
        assert_eq!(unsupported_class_file_jdk(output), Some(27));
    }

    #[test]
    fn the_explanation_names_the_jdk_and_the_remedy() {
        let explanation =
            explain_missing_reports(SPOTBUGS_ON_JDK_25).expect("the error should be recognised");
        assert!(explanation.contains("Java 25"), "got: {explanation}");
        assert!(explanation.contains("spotbugs"), "got: {explanation}");
    }

    /// A build that failed for some other reason must not be given this
    /// explanation — a confident wrong diagnosis is worse than none.
    #[test]
    fn an_unrelated_failure_is_not_explained() {
        assert!(explain_missing_reports("Could not resolve org.example:missing:1.0").is_none());
    }

    #[test]
    fn a_clean_build_is_not_explained() {
        assert!(explain_missing_reports("BUILD SUCCESSFUL in 3s").is_none());
    }
}
