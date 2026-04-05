//! Tests for TASK-CLI-311: CronTask struct and JSON persistence.

use archon_tools::cron_task::{CronTask, CronStore};
use tempfile::TempDir;

fn make_task(id: &str, cron: &str, prompt: &str, recurring: Option<bool>) -> CronTask {
    CronTask {
        id: id.to_string(),
        cron: cron.to_string(),
        prompt: prompt.to_string(),
        created_at: 1_700_000_000_000,
        recurring,
    }
}

// ---------------------------------------------------------------------------
// CronTask struct
// ---------------------------------------------------------------------------

#[test]
fn recurring_is_option_bool() {
    let one_shot = make_task("a", "* * * * *", "do it", Some(false));
    assert_eq!(one_shot.recurring, Some(false));

    let recurring = make_task("b", "* * * * *", "do it", None);
    assert_eq!(recurring.recurring, None);

    let explicit_recurring = make_task("c", "* * * * *", "do it", Some(true));
    assert_eq!(explicit_recurring.recurring, Some(true));
}

#[test]
fn cron_task_has_required_fields() {
    let task = make_task("uuid-1", "0 9 * * 1", "morning brief", None);
    assert_eq!(task.id, "uuid-1");
    assert_eq!(task.cron, "0 9 * * 1");
    assert_eq!(task.prompt, "morning brief");
    assert!(task.created_at > 0);
}

#[test]
fn cron_task_serializes_to_json() {
    let task = make_task("uuid-2", "*/5 * * * *", "check status", Some(false));
    let json = serde_json::to_value(&task).unwrap();
    assert_eq!(json["id"], "uuid-2");
    assert_eq!(json["cron"], "*/5 * * * *");
    assert_eq!(json["prompt"], "check status");
    assert_eq!(json["recurring"], false); // Some(false) → JSON false
}

#[test]
fn none_recurring_serializes_as_null_or_absent() {
    let task = make_task("uuid-3", "* * * * *", "p", None);
    let json = serde_json::to_value(&task).unwrap();
    // None → null or absent (serde skip_serializing_if = Option::is_none)
    let recurring = &json["recurring"];
    assert!(recurring.is_null() || !json.as_object().unwrap().contains_key("recurring"),
        "None recurring should serialize as null or absent, got {recurring}");
}

#[test]
fn cron_task_round_trips_json() {
    let task = make_task("uuid-4", "30 14 * * 5", "weekly task", Some(true));
    let json = serde_json::to_string(&task).unwrap();
    let restored: CronTask = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.id, task.id);
    assert_eq!(restored.cron, task.cron);
    assert_eq!(restored.prompt, task.prompt);
    assert_eq!(restored.created_at, task.created_at);
    assert_eq!(restored.recurring, task.recurring);
}

// ---------------------------------------------------------------------------
// CronStore — JSON file persistence
// ---------------------------------------------------------------------------

#[test]
fn cron_store_empty_on_missing_file() {
    let dir = TempDir::new().unwrap();
    let store = CronStore::new(dir.path().join("scheduled_tasks.json"));
    let tasks = store.load().unwrap();
    assert!(tasks.is_empty(), "missing file should return empty list");
}

#[test]
fn cron_store_write_and_read() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("scheduled_tasks.json");
    let store = CronStore::new(path);

    let task = make_task("t1", "* * * * *", "hello", None);
    store.save(&[task.clone()]).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "t1");
}

#[test]
fn cron_store_multiple_tasks() {
    let dir = TempDir::new().unwrap();
    let store = CronStore::new(dir.path().join("scheduled_tasks.json"));

    let tasks = vec![
        make_task("a", "* * * * *", "a", None),
        make_task("b", "0 * * * *", "b", Some(false)),
        make_task("c", "0 0 * * *", "c", Some(true)),
    ];
    store.save(&tasks).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 3);
}

#[test]
fn cron_store_delete_task() {
    let dir = TempDir::new().unwrap();
    let store = CronStore::new(dir.path().join("scheduled_tasks.json"));

    let tasks = vec![
        make_task("keep", "* * * * *", "keep", None),
        make_task("delete-me", "* * * * *", "delete", None),
    ];
    store.save(&tasks).unwrap();

    store.delete("delete-me").unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "keep");
}

#[test]
fn cron_store_delete_missing_id_is_noop() {
    let dir = TempDir::new().unwrap();
    let store = CronStore::new(dir.path().join("scheduled_tasks.json"));

    let tasks = vec![make_task("t1", "* * * * *", "t", None)];
    store.save(&tasks).unwrap();

    // Deleting nonexistent ID should not error
    store.delete("nonexistent").unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
}

#[test]
fn cron_store_add_task() {
    let dir = TempDir::new().unwrap();
    let store = CronStore::new(dir.path().join("scheduled_tasks.json"));

    let task = make_task("new", "* * * * *", "new task", None);
    store.add(task).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "new");
}

#[test]
fn cron_store_name_in_metadata_not_cron_task() {
    // CronTask must NOT have a 'name' field — it's stored in archon_metadata
    let dir = TempDir::new().unwrap();
    let store = CronStore::new(dir.path().join("scheduled_tasks.json"));

    let task = make_task("meta-1", "* * * * *", "p", None);
    store.add_with_name(task, Some("My Task Name")).unwrap();

    // Load raw JSON to verify structure
    let raw = std::fs::read_to_string(store.path()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

    // name must NOT be inside the tasks array items
    let task_obj = &json["tasks"][0];
    assert!(!task_obj.as_object().unwrap().contains_key("name"),
        "name must not be in CronTask struct");

    // name must be in archon_metadata
    assert_eq!(json["archon_metadata"]["meta-1"]["name"], "My Task Name");
}
