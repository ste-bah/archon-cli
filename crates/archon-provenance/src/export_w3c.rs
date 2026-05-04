use serde_json::{Map, Value, json};

use crate::record::ProvenanceEdge;
use crate::traverse::ProvenanceTrace;

pub fn export_trace_jsonld(trace: &ProvenanceTrace) -> Value {
    let mut entities = Map::new();
    for node in &trace.nodes {
        let mut entity = Map::new();
        entity.insert("prov:type".into(), json!(node.artifact_type));
        if let Some(hash) = &node.content_hash {
            entity.insert("archon:contentHash".into(), json!(hash));
        }
        entities.insert(node.artifact_id.clone(), Value::Object(entity));
    }

    json!({
        "@context": {
            "prov": "http://www.w3.org/ns/prov#",
            "archon": "https://archon.local/ns#"
        },
        "@type": "prov:Bundle",
        "archon:startArtifact": trace.start_artifact_id,
        "entity": entities,
        "wasDerivedFrom": trace.edges.iter().map(edge_json).collect::<Vec<_>>()
    })
}

fn edge_json(edge: &ProvenanceEdge) -> Value {
    let mut value = json!({
        "prov:generatedEntity": edge.from_artifact_id,
        "prov:usedEntity": edge.to_artifact_id,
        "prov:type": edge.edge_type.as_str(),
        "archon:edgeId": edge.edge_id
    });
    if edge.created_at != "synthetic"
        && let Some(object) = value.as_object_mut()
    {
        object.insert("prov:generatedAtTime".into(), json!(edge.created_at));
    }
    value
}
