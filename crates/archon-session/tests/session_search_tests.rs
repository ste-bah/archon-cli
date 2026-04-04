use std::fs;
use std::path::PathBuf;

use archon_session::metadata::{add_tag, get_tags, remove_tag};
use archon_session::search::{
    search_sessions, session_stats, SessionSearchQuery, SortField, SortOrder,
};
use archon_session::storage::SessionStore;

fn temp_db() -> (PathBuf, SessionStore) {
    let dir = std::env::temp_dir()
        .join("archon-session-search-test")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create dir");
    let path = dir.join("test.db");
    let store = SessionStore::open(&path).expect("open db");
    (dir, store)
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn search_empty_store() {
    let (dir, store) = temp_db();
    let query = SessionSearchQuery::default();
    let results = search_sessions(&store, &query).expect("search");
    assert!(results.is_empty());
    cleanup(&dir);
}

#[test]
fn search_by_branch() {
    let (dir, store) = temp_db();
    store
        .create_session("/tmp/a", Some("main"), "m1")
        .expect("create");
    store
        .create_session("/tmp/b", Some("develop"), "m1")
        .expect("create");
    store
        .create_session("/tmp/c", Some("feature-x"), "m1")
        .expect("create");

    let query = SessionSearchQuery {
        branch: Some("develop".to_string()),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].git_branch.as_deref(), Some("develop"));
    cleanup(&dir);
}

#[test]
fn search_by_directory() {
    let (dir, store) = temp_db();
    store
        .create_session("/home/user/proj-a", Some("main"), "m1")
        .expect("create");
    store
        .create_session("/home/user/proj-b", Some("main"), "m1")
        .expect("create");

    let query = SessionSearchQuery {
        directory: Some("/home/user/proj-a".to_string()),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].working_directory, "/home/user/proj-a");
    cleanup(&dir);
}

#[test]
fn search_after_date() {
    let (dir, store) = temp_db();
    // Create two sessions with known timestamps via register_session
    let old_id = "old-session";
    let new_id = "new-session";
    store
        .register_session(old_id, "/tmp", Some("main"), "m1")
        .expect("create old");
    store
        .register_session(new_id, "/tmp", Some("main"), "m1")
        .expect("create new");

    // Both sessions were just created (within seconds), so filtering with a
    // past cutoff should return both, and a future cutoff should return none.
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let query = SessionSearchQuery {
        after: Some(past),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 2);

    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let query = SessionSearchQuery {
        after: Some(future),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert!(results.is_empty());
    cleanup(&dir);
}

#[test]
fn search_before_date() {
    let (dir, store) = temp_db();
    store
        .create_session("/tmp", Some("main"), "m1")
        .expect("create");

    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let query = SessionSearchQuery {
        before: Some(future),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 1);

    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let query = SessionSearchQuery {
        before: Some(past),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert!(results.is_empty());
    cleanup(&dir);
}

#[test]
fn search_text_in_messages() {
    let (dir, store) = temp_db();
    let s1 = store
        .create_session("/tmp", Some("main"), "m1")
        .expect("create s1");
    let s2 = store
        .create_session("/tmp", Some("main"), "m1")
        .expect("create s2");

    store
        .save_message(&s1.id, 0, "hello world rust programming")
        .expect("save msg");
    store
        .save_message(&s2.id, 0, "python javascript web dev")
        .expect("save msg");

    let query = SessionSearchQuery {
        text: Some("rust".to_string()),
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, s1.id);
    cleanup(&dir);
}

#[test]
fn search_default_sort_date_desc() {
    let (dir, store) = temp_db();
    let s1 = store
        .register_session("aaa", "/tmp", Some("main"), "m1")
        .expect("create");
    // Save a message to s1 so its last_active is updated (slightly later)
    store.save_message(&s1.id, 0, "msg").expect("save");

    let s2 = store
        .register_session("bbb", "/tmp", Some("main"), "m1")
        .expect("create");
    store.save_message(&s2.id, 0, "msg").expect("save");

    let query = SessionSearchQuery::default();
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 2);
    // Newest first (bbb was created/updated after aaa)
    assert_eq!(results[0].id, "bbb");
    assert_eq!(results[1].id, "aaa");
    cleanup(&dir);
}

#[test]
fn search_sort_by_tokens() {
    let (dir, store) = temp_db();
    let s1 = store
        .register_session("low-tok", "/tmp", Some("main"), "m1")
        .expect("create");
    store
        .update_usage(&s1.id, 100, 0.01)
        .expect("update usage");

    let s2 = store
        .register_session("high-tok", "/tmp", Some("main"), "m1")
        .expect("create");
    store
        .update_usage(&s2.id, 5000, 0.50)
        .expect("update usage");

    let query = SessionSearchQuery {
        sort_by: SortField::Tokens,
        sort_order: SortOrder::Desc,
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id, "high-tok");
    assert_eq!(results[1].id, "low-tok");
    cleanup(&dir);
}

#[test]
fn search_limit() {
    let (dir, store) = temp_db();
    for i in 0..10 {
        store
            .register_session(&format!("sess-{i}"), "/tmp", Some("main"), "m1")
            .expect("create");
    }

    let query = SessionSearchQuery {
        limit: 5,
        ..Default::default()
    };
    let results = search_sessions(&store, &query).expect("search");
    assert_eq!(results.len(), 5);
    cleanup(&dir);
}

#[test]
fn add_and_get_tags() {
    let (dir, store) = temp_db();
    let session = store
        .create_session("/tmp", Some("main"), "m1")
        .expect("create");

    add_tag(&store, &session.id, "important").expect("add tag");
    add_tag(&store, &session.id, "review").expect("add tag");

    let tags = get_tags(&store, &session.id).expect("get tags");
    assert!(tags.contains(&"important".to_string()));
    assert!(tags.contains(&"review".to_string()));
    assert_eq!(tags.len(), 2);
    cleanup(&dir);
}

#[test]
fn remove_tag_works() {
    let (dir, store) = temp_db();
    let session = store
        .create_session("/tmp", Some("main"), "m1")
        .expect("create");

    add_tag(&store, &session.id, "temp").expect("add tag");
    let tags = get_tags(&store, &session.id).expect("get tags");
    assert_eq!(tags.len(), 1);

    remove_tag(&store, &session.id, "temp").expect("remove tag");
    let tags = get_tags(&store, &session.id).expect("get tags");
    assert!(tags.is_empty());
    cleanup(&dir);
}

#[test]
fn session_stats_correct() {
    let (dir, store) = temp_db();
    store
        .register_session("s1", "/tmp", Some("main"), "m1")
        .expect("create");
    store.update_usage("s1", 100, 0.01).expect("usage");

    store
        .register_session("s2", "/tmp", Some("main"), "m1")
        .expect("create");
    store.update_usage("s2", 200, 0.02).expect("usage");

    store
        .register_session("s3", "/tmp", Some("main"), "m1")
        .expect("create");
    store.update_usage("s3", 300, 0.03).expect("usage");

    let stats = session_stats(&store).expect("stats");
    assert_eq!(stats.total_sessions, 3);
    assert_eq!(stats.total_tokens, 600);
    // Each update_usage increments message_count by 1
    assert_eq!(stats.total_messages, 3);
    cleanup(&dir);
}

#[test]
fn search_query_defaults() {
    let query = SessionSearchQuery::default();
    assert_eq!(query.limit, 20);
    assert!(matches!(query.sort_by, SortField::Date));
    assert!(matches!(query.sort_order, SortOrder::Desc));
    assert!(query.branch.is_none());
    assert!(query.directory.is_none());
    assert!(query.after.is_none());
    assert!(query.before.is_none());
    assert!(query.text.is_none());
    assert!(query.tag.is_none());
}
