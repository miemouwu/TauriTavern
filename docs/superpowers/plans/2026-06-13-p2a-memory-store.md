# P2a — MemoryStore (SQLite foundation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) tracking.

**Goal:** A rusqlite-backed `MemoryStore` — the persistent Memory tables (entities, timeline, threads, state, plan, overrides, message_index, meta) + FTS5 trigram retrieval + entry-level CAS — openable at any path and fully unit-tested. No agent wiring yet (that's P2b).

**Architecture:** Infrastructure module `src-tauri/src/infrastructure/memory/`. `MemoryStore { conn: std::sync::Mutex<rusqlite::Connection> }` — sync API (async callers will use `spawn_blocking` in P2b). WAL journal mode; schema created via `CREATE TABLE IF NOT EXISTS` + a `meta(schema_version)` row. Implements the B-hybrid store validated by the macOS spike (`docs/Agent/MemoryImplementationRoadmap.md` §1.1/§7).

**Tech Stack:** Rust (`tauritavern`), `rusqlite 0.40` (`bundled` → SQLite 3.53 with FTS5), `serde_json`. Tests run from `src-tauri`: `source "$HOME/.cargo/env" && cargo test --manifest-path /Users/shoulifu/tauritavern/src-tauri/Cargo.toml <name>`.

**Schema (memory.db):**
```sql
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT) STRICT;          -- 'schema_version','watermark'
CREATE TABLE IF NOT EXISTS entities (
  id TEXT PRIMARY KEY, name TEXT, aliases TEXT, relationships TEXT, address TEXT,
  state TEXT, voice TEXT, last_confirmed_turn INTEGER, confidence TEXT,
  source_refs TEXT, version INTEGER NOT NULL DEFAULT 0) STRICT;               -- *_json cols hold JSON text
CREATE TABLE IF NOT EXISTS timeline (
  event_id TEXT PRIMARY KEY, narration_order INTEGER, diegetic_seq INTEGER,
  story_time_label TEXT, frame TEXT, status TEXT, summary TEXT,
  participants TEXT, notable_absent TEXT, source_refs TEXT) STRICT;
CREATE INDEX IF NOT EXISTS timeline_diegetic ON timeline(diegetic_seq);
CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, data TEXT) STRICT;
CREATE TABLE IF NOT EXISTS state (k TEXT PRIMARY KEY, v TEXT) STRICT;
CREATE TABLE IF NOT EXISTS plan (id TEXT PRIMARY KEY, data TEXT) STRICT;
CREATE TABLE IF NOT EXISTS overrides (
  id INTEGER PRIMARY KEY AUTOINCREMENT, target TEXT, field TEXT, value TEXT, scope_refs TEXT) STRICT;
CREATE TABLE IF NOT EXISTS message_index (msg_id TEXT PRIMARY KEY, position INTEGER, content_sha TEXT) STRICT;
CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(id UNINDEXED, name, aliases, summary, tokenize='trigram');
CREATE VIRTUAL TABLE IF NOT EXISTS timeline_fts USING fts5(event_id UNINDEXED, summary, tokenize='trigram');
```

---

## File Structure
- Create `src-tauri/src/infrastructure/memory/mod.rs` — `MemoryStore` + open/migrate + `MemoryError`.
- Create `src-tauri/src/infrastructure/memory/types.rs` — `Entity`, `TimelineEvent`, `SearchHit` (serde).
- Create `src-tauri/src/infrastructure/memory/entities.rs`, `timeline.rs`, `search.rs` — op impls on `MemoryStore` (or keep in mod.rs if small).
- Modify `src-tauri/src/infrastructure/mod.rs` — `pub mod memory;`.
- Modify `src-tauri/Cargo.toml` — add `rusqlite`.

---

## Task 1: Dependency + `MemoryStore::open` + schema migration

**Files:** `Cargo.toml`; `src/infrastructure/memory/mod.rs`; `src/infrastructure/mod.rs`.

- [ ] **Step 1: Add the dependency** — in `src-tauri/Cargo.toml` `[dependencies]`:
```toml
rusqlite = { version = "0.40", features = ["bundled"] }
```

- [ ] **Step 2: Write the failing test** (in `mod.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_in_memory_migrates_schema_and_sets_version() {
        let store = MemoryStore::open_in_memory().unwrap();
        // all core tables + fts exist
        for t in ["meta","entities","timeline","threads","state","plan","overrides","message_index","entities_fts","timeline_fts"] {
            assert!(store.table_exists(t).unwrap(), "missing table {t}");
        }
        assert_eq!(store.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
        assert_eq!(store.watermark().unwrap(), 0);
    }
}
```

- [ ] **Step 3: Run it (FAIL)** — `cargo test --manifest-path .../Cargo.toml open_in_memory_migrates`.

- [ ] **Step 4: Implement** (`mod.rs`):
```rust
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

pub mod types;

pub const MEMORY_SCHEMA_VERSION: i64 = 1;

const SCHEMA_SQL: &str = include_str!("schema.sql"); // OR inline the CREATE statements as a &str constant

#[derive(Debug)]
pub enum MemoryError {
    Sqlite(rusqlite::Error),
    VersionConflict { expected: i64, actual: i64 },
}
impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self { MemoryError::Sqlite(e) }
}
pub type MemoryResult<T> = Result<T, MemoryError>;

pub struct MemoryStore {
    conn: Mutex<Connection>,
}

impl MemoryStore {
    pub fn open(path: &Path) -> MemoryResult<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }
    pub fn open_in_memory() -> MemoryResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }
    fn init(conn: Connection) -> MemoryResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").ok(); // no-op for :memory:
        conn.execute_batch(SCHEMA_SQL)?;
        conn.execute(
            "INSERT INTO meta(k,v) VALUES('schema_version',?1) ON CONFLICT(k) DO NOTHING",
            rusqlite::params![MEMORY_SCHEMA_VERSION.to_string()],
        )?;
        conn.execute(
            "INSERT INTO meta(k,v) VALUES('watermark','0') ON CONFLICT(k) DO NOTHING", [],
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn table_exists(&self, name: &str) -> MemoryResult<bool> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name=?1 AND type IN ('table','view')",
            rusqlite::params![name], |r| r.get(0))?;
        Ok(n > 0)
    }
    pub fn schema_version(&self) -> MemoryResult<i64> { Ok(self.meta_get("schema_version")?.unwrap_or_default().parse().unwrap_or(0)) }
    pub fn watermark(&self) -> MemoryResult<i64> { Ok(self.meta_get("watermark")?.unwrap_or_default().parse().unwrap_or(0)) }
    pub fn set_watermark(&self, n: i64) -> MemoryResult<()> { self.meta_set("watermark", &n.to_string()) }

    fn meta_get(&self, k: &str) -> MemoryResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT v FROM meta WHERE k=?1", rusqlite::params![k], |r| r.get::<_,String>(0)).ok())
    }
    fn meta_set(&self, k: &str, v: &str) -> MemoryResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO meta(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=?2", rusqlite::params![k,v])?;
        Ok(())
    }
}
```
Create `src/infrastructure/memory/schema.sql` with the full schema from the plan header (the `CREATE TABLE IF NOT EXISTS …` block). Add `pub mod memory;` to `src-tauri/src/infrastructure/mod.rs`.

- [ ] **Step 5: Run it (PASS)**; **Step 6: Commit** `feat(agent-memory): MemoryStore open + schema migration (P2a)`.

---

## Task 2: Entity upsert with entry-level CAS + get

**Files:** `memory/types.rs`, `memory/mod.rs` (or `entities.rs`).

- [ ] **Step 1: Failing test:**
```rust
#[test]
fn entity_upsert_is_cas_guarded() {
    let s = MemoryStore::open_in_memory().unwrap();
    let e = Entity { id: "guyuan".into(), name: "顾远".into(), confidence: "canon".into(), ..Default::default() };
    let v1 = s.upsert_entity(&e, Some(0)).unwrap();      // create at version 0 -> 1
    assert_eq!(v1, 1);
    let got = s.get_entity("guyuan").unwrap().unwrap();
    assert_eq!(got.name, "顾远");
    assert_eq!(got.version, 1);
    // stale version is rejected
    let err = s.upsert_entity(&e, Some(0)).unwrap_err();
    assert!(matches!(err, MemoryError::VersionConflict { expected: 0, actual: 1 }));
    // correct version succeeds and bumps
    let v2 = s.upsert_entity(&e, Some(1)).unwrap();
    assert_eq!(v2, 2);
}
```

- [ ] **Step 2: Run (FAIL)**; **Step 3: Implement** — `Entity` in `types.rs` (`#[derive(Debug,Clone,Default,Serialize,Deserialize)]`, fields: `id,name,aliases:Vec<String>,relationships:Value,address:Value,state:Value,voice:String,last_confirmed_turn:i64,confidence:String,source_refs:Value,version:i64`). `upsert_entity(&self, e:&Entity, expected_version: Option<i64>) -> MemoryResult<i64>`: in a transaction, read current `version` (None if absent → treat as 0); if `expected_version` is Some and != current → `VersionConflict`; INSERT…ON CONFLICT(id) DO UPDATE setting all cols + `version=current+1`; keep `entities_fts` in sync (delete+insert the row's id/name/aliases). Return new version. `get_entity(&self,id)->MemoryResult<Option<Entity>>` parses the JSON cols. Serialize Vec/Value cols via `serde_json::to_string`.

- [ ] **Step 4: Run (PASS)**; **Step 5: Commit** `feat(agent-memory): entity upsert with CAS + get (P2a)`.

---

## Task 3: Timeline append (idempotent) + range query

- [ ] **Step 1: Failing test:**
```rust
#[test]
fn timeline_insert_is_idempotent_and_range_sorts_by_diegetic_seq() {
    let s = MemoryStore::open_in_memory().unwrap();
    let ev = |id:&str, seq:i64, sum:&str| TimelineEvent { event_id:id.into(), diegetic_seq:seq, summary:sum.into(), ..Default::default() };
    assert!(s.insert_timeline_event(&ev("e2",138,"顾远向林安安表白")).unwrap());
    assert!(s.insert_timeline_event(&ev("e1",100,"序章")).unwrap());
    assert!(!s.insert_timeline_event(&ev("e2",138,"dup")).unwrap(), "same event_id is a no-op");
    let range = s.timeline_range(0, 200).unwrap();
    assert_eq!(range.iter().map(|e| e.event_id.as_str()).collect::<Vec<_>>(), vec!["e1","e2"]); // sorted by diegetic_seq
    assert_eq!(s.timeline_range(120, 200).unwrap().len(), 1);
}
```

- [ ] **Step 2-5:** `TimelineEvent` in `types.rs` (`event_id,narration_order:i64,diegetic_seq:i64,story_time_label:String,frame:String,status:String,summary:String,participants:Vec<String>,notable_absent:Vec<String>,source_refs:Value`, Default). `insert_timeline_event(&self,ev)->MemoryResult<bool>`: `INSERT … ON CONFLICT(event_id) DO NOTHING`; returns `changes()>0`; on insert also insert into `timeline_fts(event_id,summary)`. `timeline_range(&self,from:i64,to:i64)->MemoryResult<Vec<TimelineEvent>>`: `WHERE diegetic_seq BETWEEN ?1 AND ?2 ORDER BY diegetic_seq`. Commit `feat(agent-memory): timeline append + range query (P2a)`.

---

## Task 4: FTS5 trigram search (CJK substring) over entities + timeline

- [ ] **Step 1: Failing test:**
```rust
#[test]
fn search_finds_cjk_substring_across_entities_and_timeline() {
    let s = MemoryStore::open_in_memory().unwrap();
    s.upsert_entity(&Entity{ id:"luoyunxi".into(), name:"洛云希".into(), ..Default::default() }, Some(0)).unwrap();
    s.insert_timeline_event(&TimelineEvent{ event_id:"e1".into(), diegetic_seq:1, summary:"顾远向林安安表白，洛云希在场".into(), ..Default::default() }).unwrap();
    // mid-string CJK substring
    let hits = s.search("向林安", 10).unwrap();
    assert!(hits.iter().any(|h| h.id == "e1" && h.source == SearchSource::Timeline));
    let hits2 = s.search("洛云希", 10).unwrap();
    assert!(hits2.iter().any(|h| h.source == SearchSource::Entity && h.id == "luoyunxi"));
    assert!(hits2.iter().any(|h| h.source == SearchSource::Timeline && h.id == "e1"));
}
```

- [ ] **Step 2-5:** `SearchHit { id:String, source:SearchSource, snippet:String }`, `enum SearchSource { Entity, Timeline }`. `search(&self, query:&str, limit:usize)`: query both `entities_fts` and `timeline_fts` with `MATCH ?1` using a quoted phrase (`format!("\"{}\"", query)`), UNION the hits, cap at `limit`. (Reuse the spike's verified trigram approach.) Commit `feat(agent-memory): FTS5 trigram search over entities + timeline (P2a)`.

---

## Task 5: `message_index` upsert/resolve + `clear_all` (rebuild support)

- [ ] **Step 1: Failing test:**
```rust
#[test]
fn message_index_resolves_and_clear_all_wipes_derived_data() {
    let s = MemoryStore::open_in_memory().unwrap();
    s.set_message_index("m_a", 0, "sha_a").unwrap();
    s.set_message_index("m_b", 1, "sha_b").unwrap();
    assert_eq!(s.position_of_msg("m_b").unwrap(), Some(1));
    s.upsert_entity(&Entity{ id:"x".into(), ..Default::default() }, Some(0)).unwrap();
    s.set_watermark(5).unwrap();
    s.clear_all().unwrap(); // rebuild: wipe derived tables, keep schema; watermark reset
    assert_eq!(s.get_entity("x").unwrap(), None);
    assert_eq!(s.position_of_msg("m_b").unwrap(), None);
    assert_eq!(s.watermark().unwrap(), 0);
    assert_eq!(s.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
}
```
> `Entity` needs `PartialEq` for the `== None` assertion — derive it.

- [ ] **Step 2-5:** `set_message_index(&self,msg_id,position:i64,content_sha)` (INSERT…ON CONFLICT DO UPDATE), `position_of_msg(&self,msg_id)->MemoryResult<Option<i64>>`. `clear_all(&self)`: in a transaction, `DELETE FROM` every data + fts table, reset `watermark` to 0 (keep `schema_version`). Commit `feat(agent-memory): message_index + clear_all rebuild support (P2a)`.

---

## Self-Review
- **Coverage:** roadmap §4 schema (entities/timeline/threads/state/plan/overrides/message_index/meta + FTS5) — tables created in Task 1; entities (T2), timeline (T3), search (T4), message_index + rebuild (T5). threads/state/plan tables exist (T1) but their typed ops are deferred to P2b/P3 when consolidation populates them — **noted, not built** (YAGNI for P2a; the tables exist so P2b/P3 can use them).
- **CAS:** entry-level version guard on entities (T2) mirrors the design's §15 `UPDATE … WHERE version=?`.
- **No placeholders:** every task has concrete test + impl guidance + commit. The op bodies (T2–T5 steps 3) describe exact SQL + signatures; the executor writes the rusqlite calls.
- **Deferred to P2b:** async wrapping (`spawn_blocking`), the `memory.*` tools, the non-writable persist root, B-hybrid `VACUUM INTO` placement, threads/state/plan typed ops.

## Execution Handoff
Subagent-Driven: dispatch one implementer per task (T1→T5), verify each (`cargo test` + `cargo build`), then a full-suite regression pass.
