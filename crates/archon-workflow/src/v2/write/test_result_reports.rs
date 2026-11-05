//! Fresh JUnit-compatible reports supply identities when a runner's console
//! output is quiet. Unchanged fixtures and stale reports supply no evidence.
use quick_xml::{Reader, events::Event};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

type Snapshot = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;
const MAX_REPORT_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn snapshot(root: &Path) -> Snapshot {
    let mut found = Snapshot::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                if matches!(
                    name.to_str(),
                    Some(".git" | ".archon" | "node_modules" | ".venv" | ".gradle")
                ) {
                    continue;
                }
                if name == "target" {
                    pending.extend(
                        ["surefire-reports", "failsafe-reports", "test-results"]
                            .map(|name| path.join(name)),
                    );
                } else {
                    pending.push(path);
                }
            } else if kind.is_file()
                && path.extension().is_some_and(|ext| ext == "xml")
                && is_report_path(&path)
                && let Ok(metadata) = entry.metadata()
            {
                found.insert(path, (metadata.len(), metadata.modified().ok()));
            }
        }
    }
    found
}

pub(super) fn fresh_records(root: &Path, before: &Snapshot) -> Result<String, String> {
    let mut records = String::new();
    let mut observed_bytes = 0;
    for (path, stamp) in snapshot(root) {
        if before.get(&path) == Some(&stamp) {
            continue;
        }
        observed_bytes += stamp.0;
        if observed_bytes > MAX_REPORT_BYTES {
            return Err(format!(
                "test report exceeds observation limit: {}",
                path.display()
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("test report unreadable: {}: {e}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        if !text.contains("testcase") {
            continue;
        }
        for (id, status) in
            junit(&text).map_err(|e| format!("test report invalid: {}: {e}", path.display()))?
        {
            records.push_str(&serde_json::json!({"test_id":id,"status":status}).to_string());
            records.push('\n');
        }
    }
    Ok(records)
}

fn is_report_path(path: &Path) -> bool {
    path.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some(
                "surefire-reports"
                    | "failsafe-reports"
                    | "test-results"
                    | "test-reports"
                    | "reports"
            )
        )
    }) || path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("junit") || name.starts_with("TEST-"))
}

fn junit(text: &str) -> Result<Vec<(String, &'static str)>, String> {
    let mut reader = Reader::from_str(text);
    let mut found = Vec::new();
    let mut suites = Vec::new();
    let mut test = None;
    let mut root_seen = false;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                let name = tag.local_name();
                if !root_seen {
                    root_seen = true;
                    if !matches!(name.as_ref(), b"testsuite" | b"testsuites") {
                        return Ok(Vec::new());
                    }
                }
                let mut attributes = BTreeMap::new();
                for attr in tag.attributes() {
                    let attr = attr.map_err(|e| e.to_string())?;
                    attributes.insert(
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        attr.decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| e.to_string())?
                        .into_owned(),
                    );
                }
                match name.as_ref() {
                    b"testsuite" if !empty => {
                        suites.push(attributes.get("name").cloned().unwrap_or_default())
                    }
                    b"testcase" => {
                        let name = attributes.get("name").ok_or("testcase has no name")?;
                        let class = attributes
                            .get("classname")
                            .cloned()
                            .unwrap_or_else(|| suites.join("::"));
                        let record = (format!("{class}::{name}"), "passed");
                        if empty {
                            found.push(record);
                        } else {
                            test = Some(record);
                        }
                    }
                    b"skipped" if test.is_some() => test.as_mut().unwrap().1 = "skipped",
                    b"failure" | b"error" if test.is_some() => test.as_mut().unwrap().1 = "failed",
                    _ => {}
                }
            }
            Event::End(tag) => match tag.local_name().as_ref() {
                b"testcase" => {
                    if let Some(record) = test.take() {
                        found.push(record);
                    }
                }
                b"testsuite" => {
                    suites.pop();
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if test.is_some() || !suites.is_empty() {
        return Err("unclosed test report".into());
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn junit_names_outcomes_and_escaped_names_are_preserved() {
        let records = junit(r#"<testsuite name="suite"><testcase classname="pkg.Example" name="first &amp; second"/><testcase name="failed"><failure/></testcase><testcase name="hidden"><skipped/></testcase></testsuite>"#).unwrap();
        assert_eq!(
            records,
            [
                ("pkg.Example::first & second".into(), "passed"),
                ("suite::failed".into(), "failed"),
                ("suite::hidden".into(), "skipped")
            ]
        );
        assert!(junit("<testsuite><testcase name='unclosed'>").is_err());
        assert!(
            junit("<application><testcase name='data'/></application>")
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn only_fresh_reports_supply_evidence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("reports")).unwrap();
        let stale = dir.path().join("reports/fixture.xml");
        std::fs::write(&stale, "<testsuite><testcase name='stale'/></testsuite>").unwrap();
        let before = snapshot(dir.path());
        assert!(fresh_records(dir.path(), &before).unwrap().is_empty());
        let reports = dir.path().join("module/target/surefire-reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("result.xml"),
            "<testsuite name='suite'><testcase name='fresh'/></testsuite>",
        )
        .unwrap();
        let output = fresh_records(dir.path(), &before).unwrap();
        assert!(output.contains("suite::fresh"));
        assert!(!output.contains("stale"));
    }
}
