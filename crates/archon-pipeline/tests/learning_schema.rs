use archon_pipeline::learning::schema::*;
use cozo::{DbInstance, ScriptMutability};

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

const ALL_RELATIONS: [&str; 14] = [
    "trajectories",
    "trajectory_steps",
    "patterns",
    "causal_nodes",
    "causal_links",
    "provenance_sources",
    "provenance_records",
    "desc_episodes",
    "desc_episode_metadata",
    "gnn_weights",
    "gnn_adam_state",
    "gnn_training_runs",
    "shadow_documents",
    "learning_schema_version",
];

fn get_relation_names(db: &DbInstance) -> Vec<String> {
    let result = db
        .run_script(
            "::relations",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    let name_col = result
        .headers
        .iter()
        .position(|h| h == "name")
        .expect("relations output should have 'name' column");
    result
        .rows
        .iter()
        .map(|row| row[name_col].get_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_initialize_creates_all_relations() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let names = get_relation_names(&db);
    for rel in &ALL_RELATIONS {
        assert!(
            names.contains(&rel.to_string()),
            "relation '{}' should exist after initialization",
            rel
        );
    }
}

#[test]
fn test_initialize_is_idempotent() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();
    initialize_learning_schemas(&db).unwrap();
}

#[test]
fn test_trajectories_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at] <- [
            ["traj-001", "reasoning.causal", "task-analyzer", "sess-1", ["pat-1","pat-2"], ["ctx-1"], [], 0.85, 1.2, 0.9, "/weights/w1.bin", 1700000000000, 1700000000000]
        ]
        :put trajectories {
            trajectory_id
            =>
            route,
            agent_key,
            session_id,
            patterns,
            context,
            embedding,
            quality,
            reward,
            feedback_score,
            weights_path,
            created_at,
            updated_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[trajectory_id, route, quality] :=
            *trajectories[trajectory_id, route, _, _, _, _, _, quality, _, _, _, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "traj-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "reasoning.causal");
    assert!((result.rows[0][2].get_float().unwrap() - 0.85).abs() < f64::EPSILON);
}

#[test]
fn test_trajectory_steps_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[step_id, trajectory_id, step_index, action, observation, reward, timestamp] <- [
            ["step-001", "traj-001", 0, "analyze_code", "found 3 issues", 0.7, 1700000000000]
        ]
        :put trajectory_steps {
            step_id
            =>
            trajectory_id,
            step_index,
            action,
            observation,
            reward,
            timestamp
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[step_id, trajectory_id, action] :=
            *trajectory_steps[step_id, trajectory_id, _, action, _, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "step-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "traj-001");
    assert_eq!(result.rows[0][2].get_str().unwrap(), "analyze_code");
}

#[test]
fn test_patterns_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[pattern_id, pattern_type, description, embedding, frequency, confidence, created_at, updated_at] <- [
            ["pat-001", "behavioral", "retry on transient failure", [0.1, 0.2, 0.3], 12, 0.92, 1700000000000, 1700000000000]
        ]
        :put patterns {
            pattern_id
            =>
            pattern_type,
            description,
            embedding,
            frequency,
            confidence,
            created_at,
            updated_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[pattern_id, pattern_type, confidence] :=
            *patterns[pattern_id, pattern_type, _, _, _, confidence, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "pat-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "behavioral");
    assert!((result.rows[0][2].get_float().unwrap() - 0.92).abs() < f64::EPSILON);
}

#[test]
fn test_causal_nodes_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[node_id, label, node_type, probability, evidence_count, created_at] <- [
            ["cn-001", "build_failure", "event", 0.75, 5, 1700000000000]
        ]
        :put causal_nodes {
            node_id
            =>
            label,
            node_type,
            probability,
            evidence_count,
            created_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[node_id, label, probability] :=
            *causal_nodes[node_id, label, _, probability, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "cn-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "build_failure");
    assert!((result.rows[0][2].get_float().unwrap() - 0.75).abs() < f64::EPSILON);
}

#[test]
fn test_causal_links_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[link_id, source_ids, target_id, strength, link_type, created_at] <- [
            ["cl-001", ["cn-001", "cn-002"], "cn-003", 0.88, "causes", 1700000000000]
        ]
        :put causal_links {
            link_id
            =>
            source_ids,
            target_id,
            strength,
            link_type,
            created_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[link_id, target_id, strength] :=
            *causal_links[link_id, _, target_id, strength, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "cl-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "cn-003");
    assert!((result.rows[0][2].get_float().unwrap() - 0.88).abs() < f64::EPSILON);
}

#[test]
fn test_provenance_sources_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[source_id, source_type, uri, trust_score, last_verified, created_at] <- [
            ["ps-001", "api", "https://docs.rs/cozo", 0.95, 1700000000000, 1700000000000]
        ]
        :put provenance_sources {
            source_id
            =>
            source_type,
            uri,
            trust_score,
            last_verified,
            created_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[source_id, source_type, trust_score] :=
            *provenance_sources[source_id, source_type, _, trust_score, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "ps-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "api");
    assert!((result.rows[0][2].get_float().unwrap() - 0.95).abs() < f64::EPSILON);
}

#[test]
fn test_provenance_records_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[record_id, source_id, entity_id, entity_type, derivation_chain, confidence, created_at] <- [
            ["pr-001", "ps-001", "pat-001", "pattern", ["step-1", "step-2"], 0.87, 1700000000000]
        ]
        :put provenance_records {
            record_id
            =>
            source_id,
            entity_id,
            entity_type,
            derivation_chain,
            confidence,
            created_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[record_id, entity_type, confidence] :=
            *provenance_records[record_id, _, _, entity_type, _, confidence, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "pr-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "pattern");
    assert!((result.rows[0][2].get_float().unwrap() - 0.87).abs() < f64::EPSILON);
}

#[test]
fn test_desc_episodes_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[episode_id, session_id, description, outcome, reward, tags, created_at] <- [
            ["ep-001", "sess-1", "Refactored auth module", "success", 1.0, ["refactor", "auth"], 1700000000000]
        ]
        :put desc_episodes {
            episode_id
            =>
            session_id,
            description,
            outcome,
            reward,
            tags,
            created_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[episode_id, description, reward] :=
            *desc_episodes[episode_id, _, description, _, reward, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "ep-001");
    assert_eq!(
        result.rows[0][1].get_str().unwrap(),
        "Refactored auth module"
    );
    assert!((result.rows[0][2].get_float().unwrap() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_gnn_weights_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    // Use :replace with all 11 columns (keys: layer_id, version; values: 9 columns)
    // Bytes columns get empty lists as placeholders
    let insert = r#"
        ?[layer_id, version, in_dim, out_dim, initialization, seed, weights_blob, bias_blob, norm_l2, has_nan, saved_at_ms] <- [
            ["gnn_embed", 1, 2, 2, "xavier_uniform", 42, [], [], 0.5, false, 1700000000000]
        ]
        :replace gnn_weights {
            layer_id, version
            =>
            in_dim,
            out_dim,
            initialization,
            seed,
            weights_blob,
            bias_blob,
            norm_l2,
            has_nan,
            saved_at_ms
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[layer_id, version, in_dim, out_dim] :=
            *gnn_weights[layer_id, version, in_dim, out_dim, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "gnn_embed");
    assert_eq!(result.rows[0][1].get_int().unwrap(), 1);
    assert_eq!(result.rows[0][2].get_int().unwrap(), 2);
    assert_eq!(result.rows[0][3].get_int().unwrap(), 2);
}

#[test]
fn test_training_history_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[run_id, started_at_ms, completed_at_ms, trigger_reason, samples_processed, epochs_completed, final_loss, best_loss, weight_version_before, weight_version_after, rolled_back, error] <- [
            ["tr-001", 1700000000000, 1700003600000, "schedule", 640, 5, 0.3, 0.25, 1, 2, false, null]
        ]
        :put gnn_training_runs {
            run_id
            =>
            started_at_ms,
            completed_at_ms,
            trigger_reason,
            samples_processed,
            epochs_completed,
            final_loss,
            best_loss,
            weight_version_before,
            weight_version_after,
            rolled_back,
            error
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[run_id, trigger_reason, final_loss, rolled_back] :=
            *gnn_training_runs[run_id, _, _, trigger_reason, _, _, final_loss, _, _, _, rolled_back, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "tr-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "schedule");
    assert!((result.rows[0][2].get_float().unwrap() - 0.3).abs() < f64::EPSILON);
    assert!(!result.rows[0][3].get_bool().unwrap());
}

#[test]
fn test_shadow_documents_crud() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let insert = r#"
        ?[doc_id, original_id, shadow_type, content, metadata, created_at, updated_at] <- [
            ["sd-001", "orig-001", "summary", "Condensed version of the design doc", '{"version":1}', 1700000000000, 1700000000000]
        ]
        :put shadow_documents {
            doc_id
            =>
            original_id,
            shadow_type,
            content,
            metadata,
            created_at,
            updated_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[doc_id, shadow_type, content] :=
            *shadow_documents[doc_id, _, shadow_type, content, _, _, _]
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "sd-001");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "summary");
    assert_eq!(
        result.rows[0][2].get_str().unwrap(),
        "Condensed version of the design doc"
    );
}

#[test]
fn test_verify_learning_schemas() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let report = verify_learning_schemas(&db).unwrap();

    assert_eq!(
        report.present.len(),
        14,
        "all 14 relations should be present"
    );
    assert_eq!(
        report.missing.len(),
        0,
        "no relations should be missing after initialization"
    );

    for rel in &ALL_RELATIONS {
        assert!(
            report.present.contains(&rel.to_string()),
            "relation '{}' should be in present list",
            rel
        );
    }
}

#[test]
fn test_verify_on_empty_db() {
    let db = mem_db();

    let report = verify_learning_schemas(&db).unwrap();

    assert_eq!(
        report.present.len(),
        0,
        "no relations should be present on empty db"
    );
    assert_eq!(
        report.missing.len(),
        14,
        "all 14 relations should be missing on empty db"
    );

    for rel in &ALL_RELATIONS {
        assert!(
            report.missing.contains(&rel.to_string()),
            "relation '{}' should be in missing list on empty db",
            rel
        );
    }
}

#[test]
fn test_key_constraint_trajectories() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    // First insert with :put
    let insert1 = r#"
        ?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at] <- [
            ["traj-dup", "route.a", "agent-1", "sess-1", [], [], [], 0.5, 0.5, 0.5, "/w1.bin", 1700000000000, 1700000000000]
        ]
        :put trajectories {
            trajectory_id => route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at
        }
    "#;
    db.run_script(insert1, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    // Second :put with same key should upsert (overwrite)
    let insert2 = r#"
        ?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at] <- [
            ["traj-dup", "route.b", "agent-2", "sess-2", [], [], [], 0.9, 0.9, 0.9, "/w2.bin", 1700000000000, 1700000000000]
        ]
        :put trajectories {
            trajectory_id => route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at
        }
    "#;
    db.run_script(insert2, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    // Should have exactly one row
    let count_query = r#"
        ?[count(trajectory_id)] := *trajectories[trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(count_query, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(result.rows[0][0].get_int().unwrap(), 1);

    // Verify it was overwritten with second values
    let check = r#"
        ?[route] := *trajectories["traj-dup", route, _, _, _, _, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(check, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(result.rows[0][0].get_str().unwrap(), "route.b");

    // :insert with duplicate key should error
    let insert_dup = r#"
        ?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at] <- [
            ["traj-dup", "route.c", "agent-3", "sess-3", [], [], [], 0.1, 0.1, 0.1, "/w3.bin", 1700000000000, 1700000000000]
        ]
        :insert trajectories {
            trajectory_id => route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at
        }
    "#;
    let err = db.run_script(insert_dup, Default::default(), ScriptMutability::Mutable);
    assert!(err.is_err(), ":insert with duplicate key should fail");
}
