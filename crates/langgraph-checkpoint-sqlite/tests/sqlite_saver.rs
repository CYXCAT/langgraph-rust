use std::collections::BTreeMap;

use langgraph_checkpoint::{Checkpoint, CheckpointError, CheckpointSaver, PendingWrite};
use langgraph_checkpoint_sqlite::SqliteSaver;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn sqlite_saver_put_get_list_roundtrip() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("checkpoints.db");
    let saver = SqliteSaver::new(&db_path).unwrap();
    let checkpoint = Checkpoint {
        thread_id: "thread-1".to_string(),
        checkpoint_id: "cp-1".to_string(),
        state: BTreeMap::from([(String::from("text"), json!("ab"))]),
        versions_seen: BTreeMap::from([(String::from("text"), 2)]),
        pending_writes: vec![PendingWrite {
            node: "node_b".to_string(),
            patch: BTreeMap::from([(String::from("text"), json!("b"))]),
        }],
    };

    saver.put(checkpoint.clone()).unwrap();

    let loaded = saver.get("thread-1", "cp-1").unwrap();
    assert_eq!(loaded, Some(checkpoint.clone()));

    let listed = saver.list("thread-1").unwrap();
    assert_eq!(listed, vec![checkpoint]);
}

#[test]
fn sqlite_saver_rejects_duplicate_checkpoint_id_within_thread() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("checkpoints.db");
    let saver = SqliteSaver::new(&db_path).unwrap();

    let checkpoint = Checkpoint {
        thread_id: "thread-1".to_string(),
        checkpoint_id: "cp-1".to_string(),
        state: BTreeMap::new(),
        versions_seen: BTreeMap::new(),
        pending_writes: vec![],
    };

    saver.put(checkpoint.clone()).unwrap();
    let err = saver.put(checkpoint).unwrap_err();
    assert!(matches!(err, CheckpointError::Conflict(_)));
}
