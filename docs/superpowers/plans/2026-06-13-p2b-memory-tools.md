# P2b — `memory.*` agent tools + persist integration

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Checkbox steps.

**Goal:** Make the P2a `MemoryStore` usable by the agent: a per-chat store provider (B-hybrid placement) + the `memory.propose / search / read / timeline` host tools wired into the registry + dispatcher. Single-agent this round (the agent self-reports via `memory.propose`); automatic consolidation is P3.

**Architecture:**
- `MemoryStoreProvider` (`infrastructure/memory/provider.rs`): opens + caches one `MemoryStore` per `stable_chat_id` at `<data_root>/<user>/agent-memory/<stable_chat_id>/memory.db` — **outside** the per-run workspace copy (B-hybrid; also means the agent can't `workspace_write_file` it, satisfying the §15 model-unwritable requirement). Async API via `spawn_blocking` (MemoryStore is sync).
- `memory.*` tools (`agent_tools/memory/`): handlers operate on a resolved `Arc<MemoryStore>`; mirror existing handler shape `async fn(…, call) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError>`.
- Dispatcher: new `match` arms; the dispatcher resolves the run's `stable_chat_id` and asks the provider for the store.

**Tech:** Rust (`tauritavern`), `tokio::task::spawn_blocking`. Tests from `src-tauri`.

**Deferred to P3:** automatic watermark-driven consolidation that POPULATES the store from the conversation; `VACUUM INTO` snapshot-at-commit for run-rollback atomicity (P2b persists memory.db directly via its own WAL; run-rollback consistency relies on P3's idempotent re-processing). `memory.rebuild` tool.

---

## Task 1: `MemoryStoreProvider` (per-chat open/cache + async)

**Files:** create `src-tauri/src/infrastructure/memory/provider.rs`; `mod provider;` + `pub use` in `memory/mod.rs`.

- [ ] **Step 1 — failing test** (in `provider.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn provider_opens_one_store_per_chat_at_expected_path() {
        let tmp = std::env::temp_dir().join(format!("mem-prov-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let p = MemoryStoreProvider::new(tmp.clone());
        let a = p.get_or_open("chat_1").await.unwrap();
        let b = p.get_or_open("chat_1").await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b), "same chat returns the cached store");
        let c = p.get_or_open("chat_2").await.unwrap();
        assert!(!std::sync::Arc::ptr_eq(&a, &c));
        assert!(tmp.join("chat_1").join("memory.db").exists(), "db created at <base>/<chat>/memory.db");
        // store is usable through the provider
        a.set_watermark(7).unwrap();
        assert_eq!(p.get_or_open("chat_1").await.unwrap().watermark().unwrap(), 7);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
```

- [ ] **Step 2 — run (FAIL). Step 3 — implement:**
```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{MemoryResult, MemoryStore};

/// Opens + caches one MemoryStore per stable_chat_id, under `base_dir/<chat>/memory.db`.
/// base_dir should be OUTSIDE the per-run workspace copy (B-hybrid).
pub struct MemoryStoreProvider {
    base_dir: PathBuf,
    cache: Mutex<HashMap<String, Arc<MemoryStore>>>,
}

impl MemoryStoreProvider {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir, cache: Mutex::new(HashMap::new()) }
    }

    pub async fn get_or_open(&self, stable_chat_id: &str) -> MemoryResult<Arc<MemoryStore>> {
        if let Some(store) = self.cache.lock().unwrap().get(stable_chat_id).cloned() {
            return Ok(store);
        }
        let dir = self.base_dir.join(stable_chat_id);
        let db_path = dir.join("memory.db");
        // open is blocking (rusqlite) -> spawn_blocking
        let store = tokio::task::spawn_blocking(move || -> MemoryResult<MemoryStore> {
            std::fs::create_dir_all(&dir).ok();
            MemoryStore::open(&db_path)
        })
        .await
        .expect("spawn_blocking join")?;
        let arc = Arc::new(store);
        let mut cache = self.cache.lock().unwrap();
        // double-checked: another task may have opened it
        Ok(cache.entry(stable_chat_id.to_string()).or_insert(arc).clone())
    }
}
```
> `MemoryStore::open` must be reachable (it is, from Task P2a). If `MemoryError` isn't `Send` (it wraps `rusqlite::Error`, which is `Send`), the `spawn_blocking` returns fine.

- [ ] **Step 4 — run (PASS). Step 5 — `cargo build` clean. Commit** `feat(agent-memory): per-chat MemoryStoreProvider (P2b)`.

---

## Task 2: `memory.*` tool specs + handlers

**Files:** create `agent_tools/memory/mod.rs` + `propose.rs`/`search.rs`/`read.rs`/`timeline.rs` (or one file); add `mod memory;` to `agent_tools/mod.rs`; add `memory_*_spec()` to `registry.rs` `phase2c()`.

Tool surface (model_name → behavior), each handler `async fn(store: &MemoryStore, call: &AgentToolCall) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError>` returning `AgentToolEffect::None`:
- `memory.propose` — args `{ kind: "entity"|"event", data: {...}, expectedVersion?: int }`. entity → `store.upsert_entity`; event → `store.insert_timeline_event`. Returns new version / inserted bool. (CAS conflict → recoverable tool error.)
- `memory.search` — args `{ query: string, limit?: int }` → `store.search` → list of `{id, source, snippet}`.
- `memory.read` — args `{ id: string }` → `store.get_entity` (and/or a timeline lookup) → the row.
- `memory.timeline` — args `{ from?: int, to?: int }` → `store.timeline_range`.

- [ ] **Steps:** TDD each handler against an in-memory `MemoryStore` (mirror existing handler tests + `structured_value`/`tool_error` helpers from `agent_tools/common.rs`/`structured.rs`). Define the 4 `*_spec()` fns (name `memory.search` etc., `model_name` per the gateway's tool-name convention, `description`, `input_schema` JSON). Register them in `phase2c()`. Commit `feat(agent-memory): memory.* tool specs + handlers (P2b)`.

> Resolve the exact `AgentToolResult` construction + recoverable-error helper by reading `agent_tools/workspace/read_file.rs` + `common.rs` at implementation time.

---

## Task 3: dispatcher wiring + provider injection + `run_id`→`stable_chat_id`

**Files:** `agent_tools/dispatcher.rs` (add `memory_provider: Arc<MemoryStoreProvider>` field + ctor param + `match` arms), the dispatcher construction site in `agent_runtime_service` bootstrap, and the run→`stable_chat_id` lookup.

- [ ] **Steps:**
  - Add `memory_provider: Arc<MemoryStoreProvider>` to `AgentToolDispatcher` (+ `new`).
  - In `dispatch_with_model_workspace_repository`, add arms: `memory::MEMORY_SEARCH => { let store = self.memory_for(run_id).await?; memory::search(&store, call).await? }`, etc. Implement `memory_for(run_id)`: resolve the run's `stable_chat_id` (via the run registry / session — read how `run_id` maps to the run record that carries `stable_chat_id`), then `self.memory_provider.get_or_open(&stable_chat_id).await`.
  - Construct the dispatcher with a `MemoryStoreProvider` whose `base_dir = <data_root>/<user>/agent-memory` (thread `RuntimePaths.data_root` to the bootstrap site).
  - Build must stay clean; add an integration test: dispatch a `memory.propose` (entity) then `memory.search` for it through a test dispatcher, assert the entity is found.
  - Commit `feat(agent-memory): wire memory.* into dispatcher + per-run store resolution (P2b)`.

> This is the deep-integration task — read the actual `AgentToolDispatcher::new` caller + the run-record/`stable_chat_id` accessor before editing. If `run_id`→`stable_chat_id` resolution isn't readily available inside the dispatcher, thread `stable_chat_id` into `dispatch(...)` from `tool_execution.rs` (which already has the run).

---

## Self-Review / scope
- Implements roadmap §4 `memory.*` tools + the model-unwritable store placement; uses the P2a `MemoryStore`.
- Deferred (documented): P3 consolidation (populates the store automatically), `VACUUM INTO` snapshot-at-commit, `memory.rebuild`.
- T1 + T2 are self-contained/testable now; T3 is the runtime integration (resolve exact wiring at implementation time).

## Execution
Subagent-driven, T1→T2→T3, verify each (`cargo test`/`cargo build`), full-suite regression at the end.
