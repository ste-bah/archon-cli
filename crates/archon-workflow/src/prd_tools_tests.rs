use super::*;

#[test]
fn tokens_obey_complete_identifier_and_glob_grammar() {
    for (text, expected) in [
        ("mcp__x__y", vec!["mcp__x__y"]),
        ("mcp__x__y.-", vec!["mcp__x__y"]),
        ("mcp__x__y* mcp__x mcp__x__y_", vec![]),
        ("```\nmcp__x__y\n```", vec!["mcp__x__y"]),
        ("mcp__x__a, mcp__x__b", vec!["mcp__x__a", "mcp__x__b"]),
        ("prefixmcp__x__y", vec![]),
    ] {
        assert_eq!(
            prd_mcp_tokens(text),
            expected.into_iter().map(str::to_owned).collect(),
            "{text}"
        );
    }
}

#[test]
fn first_section_accepts_optional_number_and_bounds_links() {
    for heading in [
        "## 7. Required Tools",
        "### 4.6 required tools",
        "## Required Tools",
    ] {
        let prd = format!(
            "- REQ-X-001: work\n{heading}\n| mcp__x__a | REQ-X-001 |\n## Other\n| mcp__x__b | REQ-X-001 |\n## Required Tools\n| mcp__x__c | REQ-X-001 |\n"
        );
        let decl = prd_tool_declaration(&prd);
        assert_eq!(decl.section_line, Some(2));
        assert_eq!(
            decl.tokens["mcp__x__a"],
            BTreeSet::from(["REQ-X-001".to_owned()])
        );
        assert!(decl.tokens["mcp__x__b"].is_empty());
        assert!(decl.tokens["mcp__x__c"].is_empty());
    }
}

#[test]
fn links_only_use_real_ids_in_text_or_section_rows() {
    let prd = "- REQ-X-001: call mcp__x__a\n- QX-001: call mcp__x__b\n\nREQ-X-001 mcp__x__c\n## 7. Required Tools\n| mcp__x__d | REQ-X-001, QX-001 FAKE-001 |\n| mcp__x__e | REQ-X-001-extra |\n";
    let decl = prd_tool_declaration(prd);
    assert_eq!(
        decl.tokens["mcp__x__a"],
        BTreeSet::from(["REQ-X-001".into()])
    );
    assert_eq!(decl.tokens["mcp__x__b"], BTreeSet::from(["QX-001".into()]));
    assert!(decl.tokens["mcp__x__c"].is_empty());
    assert_eq!(
        decl.tokens["mcp__x__d"],
        BTreeSet::from(["REQ-X-001".into(), "QX-001".into()])
    );
    assert!(decl.tokens["mcp__x__e"].is_empty());
    assert!(!actionable_tokens(prd).contains("mcp__x__c"));
}

#[test]
fn defects_are_sorted_and_missing_section_is_advisory_data() {
    let permitted: BTreeSet<_> = (0..7).map(|n| format!("mcp__x__t{n}")).collect();
    assert!(prd_tool_defects(&prd_tool_declaration(""), &BTreeSet::new()).is_empty());
    assert!(
        prd_tool_defects(
            &prd_tool_declaration("## Required Tools\nNone."),
            &permitted
        )
        .is_empty()
    );
    let defects = prd_tool_defects(
        &prd_tool_declaration("mcp__x__t0 mcp__x__absent"),
        &permitted,
    );
    assert_eq!(
        defects.iter().map(PrdToolDefect::code).collect::<Vec<_>>(),
        [
            "prd_tool_not_permitted",
            "prd_tool_unlinked",
            "prd_tools_undeclared"
        ]
    );
    let text = defects[2].message(Path::new("input"), Path::new("config"));
    assert!(text.contains("7 tool(s)"));
    assert!(text.contains("mcp__x__t4, ..."));
    assert!(!text.contains("mcp__x__t5"));
}

#[test]
fn configured_server_matching_ignores_ascii_case_and_requires_explicit_context() {
    for server in ["fetch", "git", "memory", "time", "sample-server"] {
        for text in [
            format!("use `{server}`"),
            format!("use {server} server"),
            format!("{server} tool"),
            format!("{server} MCP"),
            format!("{server} mcp"),
            format!("{server} Server"),
            format!("{server} TOOLs"),
            format!("mcp {server}"),
            format!("MCP server {server}"),
            format!("TOOLS {server}"),
            format!("{} MCP", server.to_uppercase()),
            format!("use `{}`", server.to_uppercase()),
            format!("{} server", server.to_uppercase()),
        ] {
            assert!(text_names_server(&text, server), "{text}");
        }
        for text in [
            format!("ordinary {server} prose"),
            format!("`{server}s`"),
            format!("`pre{server}`"),
            format!("`{server}_other`"),
            format!("ordinary {} prose", server.to_uppercase()),
        ] {
            assert!(!text_names_server(&text, server), "{text}");
        }
    }
    assert!(!text_names_server("sample_server MCP", "sample-server"));
    assert!(!text_names_server("sample server MCP", "sample-server"));
}

#[test]
fn r18_has_no_tokens_and_derives_configured_server_links() {
    let prd = include_str!("../../../tests/fixtures/r18_plan/prd.md");
    assert!(prd_mcp_tokens(prd).is_empty());
    let permitted = BTreeSet::from(["mcp__tradingview__probe".to_owned()]);
    let links = derived_server_links(prd, &permitted);
    println!("R18 derived links: {links:?}");
    assert_eq!(
        links["tradingview"],
        BTreeSet::from([
            "AC-DL-007".into(),
            "G-DL-002".into(),
            "RD-004".into(),
            "REQ-DL-033".into()
        ])
    );
    let mixed_case = derived_server_links(prd, &BTreeSet::from(["mcp__TradingView__probe".into()]));
    assert_eq!(mixed_case["TradingView"], links["tradingview"]);
    assert_eq!(
        prd_tool_defects(&prd_tool_declaration(prd), &permitted).len(),
        1
    );
}

#[test]
fn top_heading_ends_section_but_cannot_start_it() {
    let prd = "- REQ-X-001: work\n# Required Tools\nmcp__x__outside\n## Required Tools\nmcp__x__inside REQ-X-001\n# New document\nmcp__x__after REQ-X-001\n";
    let decl = prd_tool_declaration(prd);
    assert_eq!(decl.section_line, Some(4));
    assert!(decl.tokens["mcp__x__after"].is_empty());
    assert_eq!(
        actionable_tokens(prd),
        BTreeSet::from(["mcp__x__inside".to_owned()])
    );
}

#[test]
fn fenced_examples_never_create_declarations_or_actionable_calls() {
    for fence in ["```", "~~~", "````"] {
        let prd = format!(
            "- REQ-X-001: work\n\n{fence}\n## Required Tools\nmcp__x__example REQ-X-001\n{fence}\n## 2. Required Tools\nmcp__x__real REQ-X-001\n{fence}\nmcp__x__nested\n{fence}\n"
        );
        let decl = prd_tool_declaration(&prd);
        assert_eq!(decl.section_line, Some(7));
        assert_eq!(
            actionable_tokens(&prd),
            BTreeSet::from(["mcp__x__real".to_owned()])
        );
        assert!(decl.tokens["mcp__x__example"].is_empty());
        assert!(decl.tokens["mcp__x__nested"].is_empty());
    }
}

#[test]
fn ordinary_common_word_prose_creates_no_derived_links() {
    let permitted: BTreeSet<_> = ["fetch", "git", "memory", "time"]
        .into_iter()
        .map(|name| format!("mcp__{name}__probe"))
        .collect();
    let prd = "- REQ-X-001: fetch records and keep git history in memory over time.\n";
    assert!(
        derived_server_links(prd, &permitted)
            .values()
            .all(BTreeSet::is_empty)
    );
    for suffix in ["`time`", "time server", "time tool", "time MCP"] {
        let prd = format!("- REQ-X-001: use {suffix}.\n");
        assert_eq!(
            derived_server_links(&prd, &permitted)["time"],
            BTreeSet::from(["REQ-X-001".to_owned()])
        );
    }
}
