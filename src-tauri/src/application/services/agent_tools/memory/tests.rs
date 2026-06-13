use serde_json::{Value, json};

use super::super::dispatcher::AgentToolEffect;
use super::{propose, read, search, timeline};
use crate::domain::models::agent::{AgentToolCall, AgentToolResult};
use crate::infrastructure::memory::MemoryStore;

fn call(name: &str, arguments: Value) -> AgentToolCall {
    AgentToolCall {
        id: "call_test".to_string(),
        name: name.to_string(),
        arguments,
        provider_metadata: json!({}),
    }
}

fn assert_none_effect(effect: &AgentToolEffect) {
    assert!(
        matches!(effect, AgentToolEffect::None),
        "memory tools never produce a dispatch effect"
    );
}

async fn propose_entity(store: &MemoryStore, data: Value, expected_version: i64) -> AgentToolResult {
    let (result, effect) = propose(
        store,
        &call(
            "memory.propose",
            json!({ "kind": "entity", "data": data, "expectedVersion": expected_version }),
        ),
    )
    .await
    .expect("propose entity");
    assert_none_effect(&effect);
    result
}

async fn propose_event(store: &MemoryStore, data: Value) -> AgentToolResult {
    let (result, effect) = propose(
        store,
        &call("memory.propose", json!({ "kind": "event", "data": data })),
    )
    .await
    .expect("propose event");
    assert_none_effect(&effect);
    result
}

#[tokio::test]
async fn propose_entity_then_search_read_round_trips() {
    let store = MemoryStore::open_in_memory().unwrap();

    let proposed = propose_entity(
        &store,
        json!({ "id": "guyuan", "name": "顾远舟", "confidence": "canon" }),
        0,
    )
    .await;
    assert!(!proposed.is_error, "{}", proposed.content);
    assert_eq!(proposed.structured["version"], json!(1));

    // memory.search finds the freshly-proposed entity by a CJK substring. The
    // store's FTS index uses a trigram tokenizer, so the query must be at least
    // three characters wide to match (see the infra search test).
    let (found, effect) = search(&store, &call("memory.search", json!({ "query": "顾远舟" })))
        .await
        .expect("search");
    assert_none_effect(&effect);
    assert!(!found.is_error);
    let hits = found.structured["hits"].as_array().expect("hits array");
    assert!(
        hits.iter()
            .any(|hit| hit["id"] == json!("guyuan") && hit["source"] == json!("entity")),
        "search hits: {hits:?}"
    );

    // memory.read returns the entity with its version.
    let (got, effect) = read(&store, &call("memory.read", json!({ "id": "guyuan" })))
        .await
        .expect("read");
    assert_none_effect(&effect);
    assert!(!got.is_error);
    assert_eq!(got.structured["entity"]["name"], json!("顾远舟"));
    assert_eq!(got.structured["entity"]["version"], json!(1));
}

#[tokio::test]
async fn read_missing_entity_returns_null_entity() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (result, effect) = read(&store, &call("memory.read", json!({ "id": "nope" })))
        .await
        .expect("read");
    assert_none_effect(&effect);
    assert!(!result.is_error, "missing entity is not an error");
    assert_eq!(result.structured["entity"], Value::Null);
}

#[tokio::test]
async fn timeline_returns_inserted_events_in_range() {
    let store = MemoryStore::open_in_memory().unwrap();
    let first = propose_event(
        &store,
        json!({ "event_id": "e1", "diegetic_seq": 100, "summary": "序章" }),
    )
    .await;
    assert_eq!(first.structured["inserted"], json!(true));
    propose_event(
        &store,
        json!({ "event_id": "e2", "diegetic_seq": 138, "summary": "顾远向林安安表白" }),
    )
    .await;

    // Default range (omit from/to) returns the whole timeline, sorted by diegetic_seq.
    let (all, effect) = timeline(&store, &call("memory.timeline", json!({})))
        .await
        .expect("timeline");
    assert_none_effect(&effect);
    assert!(!all.is_error);
    let events = all.structured["events"].as_array().expect("events array");
    let ids = events
        .iter()
        .map(|event| event["event_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["e1", "e2"]);

    // A narrow range filters to a single event.
    let (narrow, _) = timeline(
        &store,
        &call("memory.timeline", json!({ "from": 120, "to": 200 })),
    )
    .await
    .expect("timeline narrow");
    assert_eq!(
        narrow.structured["events"].as_array().unwrap().len(),
        1,
        "only e2 falls in [120, 200]"
    );
}

#[tokio::test]
async fn propose_event_duplicate_is_idempotent_noop() {
    let store = MemoryStore::open_in_memory().unwrap();
    let data = json!({ "event_id": "e1", "diegetic_seq": 1, "summary": "序章" });
    let first = propose_event(&store, data.clone()).await;
    assert_eq!(first.structured["inserted"], json!(true));
    let second = propose_event(&store, data).await;
    assert!(!second.is_error);
    assert_eq!(
        second.structured["inserted"],
        json!(false),
        "re-inserting the same event_id is a no-op"
    );
}

#[tokio::test]
async fn propose_entity_with_stale_version_is_recoverable_conflict() {
    let store = MemoryStore::open_in_memory().unwrap();
    let data = json!({ "id": "guyuan", "name": "顾远" });
    // Create at version 0 -> 1.
    let created = propose_entity(&store, data.clone(), 0).await;
    assert_eq!(created.structured["version"], json!(1));

    // Proposing again with the now-stale expectedVersion 0 yields a recoverable
    // tool error (is_error, code memory.version_conflict) rather than a hard
    // ApplicationError, so the model can re-read and retry.
    let conflict = propose_entity(&store, data.clone(), 0).await;
    assert!(conflict.is_error, "stale version must be a tool error");
    assert_eq!(
        conflict.error_code.as_deref(),
        Some("memory.version_conflict")
    );
    assert_eq!(
        conflict.structured["error"]["code"],
        json!("memory.version_conflict")
    );

    // Retrying with the correct version bumps to 2.
    let retried = propose_entity(&store, data, 1).await;
    assert!(!retried.is_error);
    assert_eq!(retried.structured["version"], json!(2));
}

#[tokio::test]
async fn search_missing_query_is_recoverable_tool_error() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (result, effect) = search(&store, &call("memory.search", json!({})))
        .await
        .expect("search");
    assert_none_effect(&effect);
    assert!(result.is_error);
    assert_eq!(result.error_code.as_deref(), Some("tool.invalid_arguments"));
}

#[tokio::test]
async fn propose_rejects_unknown_kind_as_tool_error() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (result, effect) = propose(
        &store,
        &call(
            "memory.propose",
            json!({ "kind": "thread", "data": { "id": "x" } }),
        ),
    )
    .await
    .expect("propose");
    assert_none_effect(&effect);
    assert!(result.is_error);
    assert_eq!(result.error_code.as_deref(), Some("tool.invalid_arguments"));
}

#[tokio::test]
async fn propose_entity_rejects_malformed_data_as_tool_error() {
    let store = MemoryStore::open_in_memory().unwrap();
    // `id` must be a string; a number fails entity deserialization.
    let (result, effect) = propose(
        &store,
        &call(
            "memory.propose",
            json!({ "kind": "entity", "data": { "id": 7 } }),
        ),
    )
    .await
    .expect("propose");
    assert_none_effect(&effect);
    assert!(result.is_error);
    assert_eq!(result.error_code.as_deref(), Some("tool.invalid_arguments"));
}
