use archon_llm::providers::codex::types::ResponseStreamEvent;

fn main() {
    let fixtures = [
        serde_json::json!({"type":"response.created","response":{"id":"r"}}),
        serde_json::json!({"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"hi"}),
        serde_json::json!({"type":"response.future"}),
    ];

    for fixture in fixtures {
        match serde_json::from_value::<ResponseStreamEvent>(fixture) {
            Ok(event) => println!("OK: {event:?}"),
            Err(err) => println!("ERR: {err}"),
        }
    }
}
