use std::collections::BTreeMap;
use std::env;

use langgraph_checkpoint::{Checkpoint, CheckpointError, CheckpointSaver, PendingWrite};
use langgraph_checkpoint_postgres::PostgresSaver;
use postgres::{Client, NoTls};
use serde_json::json;
use uuid::Uuid;

fn unique_thread_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

fn test_database_url() -> Option<String> {
    env::var("LANGGRAPH_POSTGRES_TEST_URL").ok()
}

fn cleanup_thread(url: &str, thread_id: &str) {
    let mut client = Client::connect(url, NoTls).expect("connect postgres for cleanup");
    client
        .execute("DELETE FROM checkpoints WHERE thread_id = $1", &[&thread_id])
        .expect("cleanup thread checkpoints");
}

#[test]
fn postgres_saver_put_get_list_roundtrip() {
    let Some(url) = test_database_url() else {
        return;
    };
    let thread_id = unique_thread_id("thread-pg-roundtrip");
    let saver = PostgresSaver::new(url.clone()).expect("postgres saver should init");
    let checkpoint = Checkpoint {
        thread_id: thread_id.clone(),
        checkpoint_id: String::from("cp-1"),
        state: BTreeMap::from([(String::from("text"), json!("ab"))]),
        versions_seen: BTreeMap::from([(String::from("text"), 2)]),
        pending_writes: vec![PendingWrite {
            node: String::from("node_b"),
            patch: BTreeMap::from([(String::from("text"), json!("b"))]),
        }],
    };

    saver.put(checkpoint.clone()).expect("put should succeed");
    let loaded = saver
        .get(&thread_id, "cp-1")
        .expect("get should succeed")
        .expect("checkpoint should exist");
    assert_eq!(loaded, checkpoint);

    let listed = saver.list(&thread_id).expect("list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].checkpoint_id, "cp-1");

    cleanup_thread(&url, &thread_id);
}

#[test]
fn postgres_saver_rejects_duplicate_checkpoint_id_within_thread() {
    let Some(url) = test_database_url() else {
        return;
    };
    let thread_id = unique_thread_id("thread-pg-conflict");
    let saver = PostgresSaver::new(url.clone()).expect("postgres saver should init");
    let checkpoint = Checkpoint {
        thread_id: thread_id.clone(),
        checkpoint_id: String::from("cp-1"),
        state: BTreeMap::new(),
        versions_seen: BTreeMap::new(),
        pending_writes: vec![],
    };

    saver.put(checkpoint.clone()).expect("first put should succeed");
    let err = saver.put(checkpoint).expect_err("duplicate put should conflict");
    assert!(matches!(err, CheckpointError::Conflict(_)));

    cleanup_thread(&url, &thread_id);
}
