//! Issue 312: the precheck and the derived reader must accept exactly the
//! same documents. A field the reader ignores is skipped with `IgnoredAny`,
//! which checks JSON syntax only, so its contents can be deeper than the
//! recursion limit, hold a lone surrogate, invalid UTF-8 or a number out of
//! f64 range, and the document is still read. A field the reader reads is
//! refused with the same values. Every value goes into every site, and each
//! verdict is compared both ways: no false refusal and no false green.
use super::*;
use crate::command::workflow_freeze_candidate::candidate_document;
use archon_workflow::task_skeleton::TaskSkeleton;

const HEAD: &str = r#""schema_version":1,"acceptance_digest":"d""#;
const HOLE: &str = "@";

/// Raw JSON values that `Value` refuses but `IgnoredAny` skips (or, for the
/// last ones, that both read the same way but a struct field refuses).
fn values() -> Vec<(&'static str, Vec<u8>)> {
    let deep = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth)).into_bytes();
    vec![
        ("deeper than the recursion limit", deep(200)),
        ("far deeper than the recursion limit", deep(5000)),
        (
            "deep object",
            format!("{}1{}", r#"{"a":"#.repeat(200), "}".repeat(200)).into_bytes(),
        ),
        ("lone leading surrogate", br#""\ud800""#.to_vec()),
        ("lone trailing surrogate", br#""\udfff""#.to_vec()),
        (
            "invalid UTF-8",
            [b"\"".as_slice(), &[0xFF, 0xFE], b"\""].concat(),
        ),
        ("number out of f64 range", b"1e400".to_vec()),
        ("negative number out of range", b"-1e400".to_vec()),
        ("huge integer", b"184467440737095516160000".to_vec()),
        (
            "duplicate keys",
            br#"{"k":1,"k":[2],"k":"\ud800"}"#.to_vec(),
        ),
        (
            "lone surrogate in a nested key",
            br#"{"\udc00":1}"#.to_vec(),
        ),
        ("plain string", br#""x""#.to_vec()),
    ]
}

/// Document templates with one hole, and whether the reader ignores the hole.
fn sites() -> Vec<(&'static str, String, bool)> {
    let task =
        |body: &str| format!(r#"{{{HEAD},"tasks":[{{"task_id":"T","file_name":"f",{body}}}]}}"#);
    vec![
        (
            "root ignored field",
            format!(r#"{{{HEAD},"tasks":[],"zz":@}}"#),
            true,
        ),
        ("task ignored field", task(r#""zz":@"#), true),
        ("repeated ignored field", task(r#""zz":@,"zz":@"#), true),
        (
            "dependency ignored field",
            task(r#""depends_on":[{"task_id":"U","ordering_only":true,"zz":@}]"#),
            true,
        ),
        (
            "consumed ignored field",
            task(r#""depends_on":[{"task_id":"U","consumes":[{"artifact_path":"a","zz":@}]}]"#),
            true,
        ),
        (
            "unknown field of a closed struct",
            task(r#""deliverable_contracts":[{"kind":"file","artifact_path":"a","zz":@}]"#),
            false,
        ),
        ("read string field", task(r#""implements":[@]"#), false),
        (
            "read task id",
            format!(r#"{{{HEAD},"tasks":[{{"task_id":@,"file_name":"f"}}]}}"#),
            false,
        ),
        (
            "read number field",
            r#"{"schema_version":@,"acceptance_digest":"d","tasks":[]}"#.to_string(),
            false,
        ),
        (
            "extra positional field",
            format!(r#"{{{HEAD},"tasks":[["T","f",[],[],[],[],@]]}}"#),
            false,
        ),
        ("read list", format!(r#"{{{HEAD},"tasks":@}}"#), false),
    ]
}

fn fill(template: &str, value: &[u8]) -> Vec<u8> {
    let parts: Vec<_> = template.split(HOLE).map(str::as_bytes).collect();
    parts.join(value)
}

/// The precheck verdict, the derived reader's verdict, and the precheck's
/// defects for the message when they disagree.
fn verdicts(document: &[u8]) -> (bool, bool, String) {
    let defects = element_shape_defects(document, &TASK_SHAPE);
    let reader = serde_json::from_slice::<TaskSkeleton>(&candidate_document(document));
    let shown = format!(
        "precheck {:?}; reader {:?}",
        defects.iter().map(|d| &d.message).collect::<Vec<_>>(),
        reader.as_ref().map(drop).map_err(ToString::to_string)
    );
    (defects.is_empty(), reader.is_ok(), shown)
}

#[test]
fn workflow_freeze_shape_precheck_and_reader_agree_on_every_value_in_every_site() {
    let mut checked = 0;
    for (site, template, ignored) in sites() {
        for (value, bytes) in values() {
            let document = fill(&template, &bytes);
            let (precheck, reader, shown) = verdicts(&document);
            assert_eq!(precheck, reader, "{site} / {value}: {shown}");
            // A plain string is also valid where the reader reads a string.
            let string_site = matches!(site, "read string field" | "read task id");
            let expected = ignored || (value == "plain string" && string_site);
            assert_eq!(reader, expected, "{site} / {value}: {shown}");
            checked += 1;
        }
    }
    assert_eq!(checked, sites().len() * values().len());
}

/// The four documents of the issue, as written there.
#[test]
fn workflow_freeze_shape_issue_312_repros_are_accepted_like_the_reader() {
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let task = |zz: &[u8]| {
        let head = format!(r#"{{{HEAD},"tasks":[{{"task_id":"T","file_name":"f","zz":"#);
        [head.as_bytes(), zz, b"}]}"].concat()
    };
    let repros = [
        task(deep.as_bytes()),
        format!(r#"{{{HEAD},"tasks":[],"zz":{deep}}}"#).into_bytes(),
        task(br#""\ud800""#),
        task(&[b"\"".as_slice(), &[0xFF, 0xFE], b"\""].concat()),
    ];
    for document in repros {
        let (precheck, reader, shown) = verdicts(&document);
        assert!(reader, "{shown}");
        assert!(precheck, "{shown}");
    }
}

/// The controls of the issue: the same values in a read field are refused
/// by both, and the precheck says why.
#[test]
fn workflow_freeze_shape_values_in_read_fields_stay_refused() {
    for bad in [
        br#""\ud800""#.to_vec(),
        [b"\"".as_slice(), &[0xFF, 0xFE], b"\""].concat(),
    ] {
        let document = [
            format!(r#"{{{HEAD},"tasks":[{{"task_id":"#).as_bytes(),
            &bad,
            br#","file_name":"f"}]}"#,
        ]
        .concat();
        let (precheck, reader, shown) = verdicts(&document);
        assert!(!reader && !precheck, "{shown}");
    }
}

/// A later inspection of the candidate reads it as the reader does, so a
/// candidate the reader accepts never fails there. It checks no shape: what
/// the reader refuses for its shape the precheck has refused already.
#[test]
fn workflow_freeze_shape_skeleton_document_reads_what_the_reader_reads() {
    let mut read_by_both = 0;
    for (site, template, _) in sites() {
        for (value, bytes) in values() {
            let document = fill(&template, &bytes);
            if serde_json::from_slice::<TaskSkeleton>(&document).is_ok() {
                let read = skeleton_document(&document);
                assert!(read.is_ok(), "{site} / {value}: {read:?}");
                read_by_both += 1;
            }
        }
    }
    assert!(read_by_both > values().len() * 4, "{read_by_both}");
    let document = fill(&sites()[1].1, br#""\ud800""#);
    let read = skeleton_document(&document).expect("read");
    assert_eq!(read["tasks"][0]["task_id"], "T");
    assert!(read["tasks"][0]["zz"].is_null(), "{read}");
}
