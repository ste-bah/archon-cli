use archon_pipeline::coding::evidence::{
    ApiContract, CallEdge, CallGraph, Entrypoint, EvidenceFact, EvidencePack, FileLineRef,
    TestReference, load_evidence_pack, save_evidence_pack, validate_evidence_pack,
};
use archon_pipeline::coding::AGENTS;
use tempfile::TempDir;

fn valid_file_line_ref() -> FileLineRef {
    FileLineRef {
        file: "src/main.rs".to_string(),
        line: 42,
    }
}

fn valid_fact() -> EvidenceFact {
    EvidenceFact {
        claim: "main function exists".to_string(),
        evidence: valid_file_line_ref(),
        tool_used: "leann-search".to_string(),
        verified_at: "2026-04-07T10:00:00Z".to_string(),
    }
}

fn empty_pack() -> EvidencePack {
    EvidencePack {
        facts: vec![],
        call_graph: CallGraph { edges: vec![] },
        existing_tests: vec![],
        entrypoints: vec![],
        api_contracts: vec![],
    }
}

fn well_formed_pack() -> EvidencePack {
    EvidencePack {
        facts: vec![valid_fact()],
        call_graph: CallGraph {
            edges: vec![CallEdge {
                caller: FileLineRef {
                    file: "src/lib.rs".to_string(),
                    line: 10,
                },
                callee: FileLineRef {
                    file: "src/util.rs".to_string(),
                    line: 20,
                },
                function_name: "helper".to_string(),
            }],
        },
        existing_tests: vec![TestReference {
            test_file: "tests/test_main.rs".to_string(),
            test_function: "test_hello".to_string(),
            covers_module: "main".to_string(),
        }],
        entrypoints: vec![Entrypoint {
            file: "src/main.rs".to_string(),
            function: "main".to_string(),
            entrypoint_type: "binary".to_string(),
        }],
        api_contracts: vec![ApiContract {
            endpoint: "/api/v1/health".to_string(),
            method: "GET".to_string(),
            request_type: None,
            response_type: Some("HealthResponse".to_string()),
            file: "src/routes.rs".to_string(),
            line: 15,
        }],
    }
}

// 1. Valid EvidencePack serializes/deserializes via serde_json
#[test]
fn test_evidence_pack_serde_roundtrip() {
    let pack = well_formed_pack();
    let json = serde_json::to_string_pretty(&pack).expect("serialize");
    let deserialized: EvidencePack = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized.facts.len(), 1);
    assert_eq!(deserialized.facts[0].claim, "main function exists");
    assert_eq!(deserialized.call_graph.edges.len(), 1);
    assert_eq!(deserialized.existing_tests.len(), 1);
    assert_eq!(deserialized.entrypoints.len(), 1);
    assert_eq!(deserialized.api_contracts.len(), 1);
}

// 2. validate_evidence_pack() accepts a well-formed pack with valid facts
#[test]
fn test_validate_well_formed_pack() {
    let pack = well_formed_pack();
    assert!(validate_evidence_pack(&pack).is_ok());
}

// 3. Fact with empty file → validation failure listing the bad claim
#[test]
fn test_validate_empty_file_fact() {
    let pack = EvidencePack {
        facts: vec![EvidenceFact {
            claim: "bad claim".to_string(),
            evidence: FileLineRef {
                file: "".to_string(),
                line: 10,
            },
            tool_used: "grep".to_string(),
            verified_at: "2026-04-07T10:00:00Z".to_string(),
        }],
        ..empty_pack()
    };
    let err = validate_evidence_pack(&pack).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].claim, "bad claim");
    assert!(err[0].reason.contains("file"));
}

// 4. Fact with line == 0 → validation failure listing the bad claim
#[test]
fn test_validate_line_zero_fact() {
    let pack = EvidencePack {
        facts: vec![EvidenceFact {
            claim: "zero line claim".to_string(),
            evidence: FileLineRef {
                file: "src/lib.rs".to_string(),
                line: 0,
            },
            tool_used: "grep".to_string(),
            verified_at: "2026-04-07T10:00:00Z".to_string(),
        }],
        ..empty_pack()
    };
    let err = validate_evidence_pack(&pack).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].claim, "zero line claim");
    assert!(err[0].reason.contains("line"));
}

// 5. Multiple invalid facts → all errors reported
#[test]
fn test_validate_multiple_invalid_facts() {
    let pack = EvidencePack {
        facts: vec![
            EvidenceFact {
                claim: "empty file claim".to_string(),
                evidence: FileLineRef {
                    file: "".to_string(),
                    line: 5,
                },
                tool_used: "grep".to_string(),
                verified_at: "2026-04-07T10:00:00Z".to_string(),
            },
            EvidenceFact {
                claim: "zero line claim".to_string(),
                evidence: FileLineRef {
                    file: "src/lib.rs".to_string(),
                    line: 0,
                },
                tool_used: "grep".to_string(),
                verified_at: "2026-04-07T10:00:00Z".to_string(),
            },
            EvidenceFact {
                claim: "both bad claim".to_string(),
                evidence: FileLineRef {
                    file: "  ".to_string(),
                    line: 0,
                },
                tool_used: "grep".to_string(),
                verified_at: "2026-04-07T10:00:00Z".to_string(),
            },
        ],
        ..empty_pack()
    };
    let err = validate_evidence_pack(&pack).unwrap_err();
    // empty file claim (1) + zero line claim (1) + both bad claim (2) = 4
    assert_eq!(err.len(), 4);
    let claims: Vec<&str> = err.iter().map(|e| e.claim.as_str()).collect();
    assert!(claims.contains(&"empty file claim"));
    assert!(claims.contains(&"zero line claim"));
    assert!(claims.contains(&"both bad claim"));
}

// 6. Empty facts vec is valid
#[test]
fn test_validate_empty_facts_is_valid() {
    let pack = empty_pack();
    assert!(validate_evidence_pack(&pack).is_ok());
}

// 7. CallGraph serialization with CallEdge entries
#[test]
fn test_call_graph_serialization() {
    let graph = CallGraph {
        edges: vec![
            CallEdge {
                caller: FileLineRef {
                    file: "src/a.rs".to_string(),
                    line: 1,
                },
                callee: FileLineRef {
                    file: "src/b.rs".to_string(),
                    line: 2,
                },
                function_name: "foo".to_string(),
            },
            CallEdge {
                caller: FileLineRef {
                    file: "src/b.rs".to_string(),
                    line: 3,
                },
                callee: FileLineRef {
                    file: "src/c.rs".to_string(),
                    line: 4,
                },
                function_name: "bar".to_string(),
            },
        ],
    };
    let json = serde_json::to_string(&graph).expect("serialize");
    let deser: CallGraph = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.edges.len(), 2);
    assert_eq!(deser.edges[0].function_name, "foo");
    assert_eq!(deser.edges[1].function_name, "bar");
}

// 8. TestReference serialization
#[test]
fn test_test_reference_serialization() {
    let tr = TestReference {
        test_file: "tests/integration.rs".to_string(),
        test_function: "test_api".to_string(),
        covers_module: "api".to_string(),
    };
    let json = serde_json::to_string(&tr).expect("serialize");
    let deser: TestReference = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.test_file, "tests/integration.rs");
    assert_eq!(deser.test_function, "test_api");
    assert_eq!(deser.covers_module, "api");
}

// 9. Entrypoint serialization
#[test]
fn test_entrypoint_serialization() {
    let ep = Entrypoint {
        file: "src/main.rs".to_string(),
        function: "main".to_string(),
        entrypoint_type: "binary".to_string(),
    };
    let json = serde_json::to_string(&ep).expect("serialize");
    let deser: Entrypoint = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.file, "src/main.rs");
    assert_eq!(deser.function, "main");
    assert_eq!(deser.entrypoint_type, "binary");
}

// 10. ApiContract serialization
#[test]
fn test_api_contract_serialization() {
    let ac = ApiContract {
        endpoint: "/api/users".to_string(),
        method: "POST".to_string(),
        request_type: Some("CreateUserRequest".to_string()),
        response_type: Some("UserResponse".to_string()),
        file: "src/routes/users.rs".to_string(),
        line: 42,
    };
    let json = serde_json::to_string(&ac).expect("serialize");
    let deser: ApiContract = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.endpoint, "/api/users");
    assert_eq!(deser.method, "POST");
    assert_eq!(deser.request_type.as_deref(), Some("CreateUserRequest"));
    assert_eq!(deser.response_type.as_deref(), Some("UserResponse"));
    assert_eq!(deser.file, "src/routes/users.rs");
    assert_eq!(deser.line, 42);
}

// 11. Persistence: save/load roundtrip via tempfile
#[test]
fn test_save_load_evidence_pack_roundtrip() {
    let pack = well_formed_pack();
    let tmp = TempDir::new().expect("create tempdir");
    save_evidence_pack(&pack, tmp.path()).expect("save");
    let loaded = load_evidence_pack(tmp.path()).expect("load");
    assert_eq!(loaded.facts.len(), 1);
    assert_eq!(loaded.facts[0].claim, "main function exists");
    assert_eq!(loaded.call_graph.edges.len(), 1);
    assert_eq!(loaded.existing_tests.len(), 1);
    assert_eq!(loaded.entrypoints.len(), 1);
    assert_eq!(loaded.api_contracts.len(), 1);
}

// 12. context-gatherer agent updated: check that AGENTS contains context-gatherer with evidence-related description
#[test]
fn test_context_gatherer_agent_has_evidence_description() {
    let agent = AGENTS
        .iter()
        .find(|a| a.key == "context-gatherer")
        .expect("context-gatherer agent should exist");
    assert!(
        agent.description.contains("EvidencePack"),
        "context-gatherer description should mention EvidencePack, got: {}",
        agent.description
    );
}

// 13. FileLineRef with valid data passes validation
#[test]
fn test_file_line_ref_valid_data_passes() {
    let pack = EvidencePack {
        facts: vec![EvidenceFact {
            claim: "valid ref claim".to_string(),
            evidence: FileLineRef {
                file: "src/lib.rs".to_string(),
                line: 100,
            },
            tool_used: "leann-search".to_string(),
            verified_at: "2026-04-07T10:00:00Z".to_string(),
        }],
        ..empty_pack()
    };
    assert!(validate_evidence_pack(&pack).is_ok());
}

// 14. Mixed valid/invalid facts → only invalid ones reported
#[test]
fn test_validate_mixed_facts_only_invalid_reported() {
    let pack = EvidencePack {
        facts: vec![
            valid_fact(), // valid
            EvidenceFact {
                claim: "invalid one".to_string(),
                evidence: FileLineRef {
                    file: "".to_string(),
                    line: 5,
                },
                tool_used: "grep".to_string(),
                verified_at: "2026-04-07T10:00:00Z".to_string(),
            },
            valid_fact(), // valid
        ],
        ..empty_pack()
    };
    let err = validate_evidence_pack(&pack).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].claim, "invalid one");
}

// Additional tests for completeness

// 15. EvidenceValidationError Display impl
#[test]
fn test_evidence_validation_error_display() {
    let pack = EvidencePack {
        facts: vec![EvidenceFact {
            claim: "test claim".to_string(),
            evidence: FileLineRef {
                file: "".to_string(),
                line: 1,
            },
            tool_used: "grep".to_string(),
            verified_at: "2026-04-07T10:00:00Z".to_string(),
        }],
        ..empty_pack()
    };
    let err = validate_evidence_pack(&pack).unwrap_err();
    let msg = format!("{}", err[0]);
    assert!(msg.contains("Unsourced claim"));
    assert!(msg.contains("test claim"));
}

// 16. ApiContract with None request/response types
#[test]
fn test_api_contract_optional_fields_none() {
    let ac = ApiContract {
        endpoint: "/health".to_string(),
        method: "GET".to_string(),
        request_type: None,
        response_type: None,
        file: "src/routes.rs".to_string(),
        line: 1,
    };
    let json = serde_json::to_string(&ac).expect("serialize");
    let deser: ApiContract = serde_json::from_str(&json).expect("deserialize");
    assert!(deser.request_type.is_none());
    assert!(deser.response_type.is_none());
}

// 17. Load from nonexistent path fails
#[test]
fn test_load_evidence_pack_nonexistent_path_fails() {
    let result = load_evidence_pack(std::path::Path::new("/tmp/nonexistent_dir_12345"));
    assert!(result.is_err());
}

// 18. FileLineRef serialization
#[test]
fn test_file_line_ref_serialization() {
    let flr = FileLineRef {
        file: "src/main.rs".to_string(),
        line: 99,
    };
    let json = serde_json::to_string(&flr).expect("serialize");
    let deser: FileLineRef = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.file, "src/main.rs");
    assert_eq!(deser.line, 99);
}

// 19. EvidenceFact serialization
#[test]
fn test_evidence_fact_serialization() {
    let fact = valid_fact();
    let json = serde_json::to_string(&fact).expect("serialize");
    let deser: EvidenceFact = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.claim, "main function exists");
    assert_eq!(deser.tool_used, "leann-search");
    assert_eq!(deser.evidence.file, "src/main.rs");
    assert_eq!(deser.evidence.line, 42);
}

// 20. Whitespace-only file is treated as empty
#[test]
fn test_validate_whitespace_only_file_is_empty() {
    let pack = EvidencePack {
        facts: vec![EvidenceFact {
            claim: "whitespace file".to_string(),
            evidence: FileLineRef {
                file: "   ".to_string(),
                line: 5,
            },
            tool_used: "grep".to_string(),
            verified_at: "2026-04-07T10:00:00Z".to_string(),
        }],
        ..empty_pack()
    };
    let err = validate_evidence_pack(&pack).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].claim, "whitespace file");
}
