#![allow(dead_code)] // P2a foundation; wired into agent tools in P2b
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

pub mod entities;
pub mod index;
pub mod search;
pub mod timeline;
pub mod types;

pub const MEMORY_SCHEMA_VERSION: i64 = 1;

const SCHEMA_SQL: &str = include_str!("schema.sql");

#[derive(Debug)]
pub enum MemoryError {
    Sqlite(rusqlite::Error),
    Serde(serde_json::Error),
    VersionConflict { expected: i64, actual: i64 },
}
impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self {
        MemoryError::Sqlite(e)
    }
}
impl From<serde_json::Error> for MemoryError {
    fn from(e: serde_json::Error) -> Self {
        MemoryError::Serde(e)
    }
}
pub type MemoryResult<T> = Result<T, MemoryError>;

pub struct MemoryStore {
    conn: Mutex<Connection>,
}

impl MemoryStore {
    pub fn open(path: &Path) -> MemoryResult<Self> {
        Self::init(Connection::open(path)?)
    }
    pub fn open_in_memory() -> MemoryResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> MemoryResult<Self> {
        let _ = conn.pragma_update(None, "journal_mode", "WAL"); // no-op for in-memory
        conn.execute_batch(SCHEMA_SQL)?;
        conn.execute(
            "INSERT INTO meta(k,v) VALUES('schema_version',?1) ON CONFLICT(k) DO NOTHING",
            rusqlite::params![MEMORY_SCHEMA_VERSION.to_string()],
        )?;
        conn.execute(
            "INSERT INTO meta(k,v) VALUES('watermark','0') ON CONFLICT(k) DO NOTHING",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn table_exists(&self, name: &str) -> MemoryResult<bool> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name=?1 AND type IN ('table','view')",
            rusqlite::params![name],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
    pub fn schema_version(&self) -> MemoryResult<i64> {
        Ok(self
            .meta_get("schema_version")?
            .unwrap_or_default()
            .parse()
            .unwrap_or(0))
    }
    pub fn watermark(&self) -> MemoryResult<i64> {
        Ok(self
            .meta_get("watermark")?
            .unwrap_or_default()
            .parse()
            .unwrap_or(0))
    }
    pub fn set_watermark(&self, n: i64) -> MemoryResult<()> {
        self.meta_set("watermark", &n.to_string())
    }

    fn meta_get(&self, k: &str) -> MemoryResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT v FROM meta WHERE k=?1",
                rusqlite::params![k],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }
    fn meta_set(&self, k: &str, v: &str) -> MemoryResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO meta(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=?2",
            rusqlite::params![k, v],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::types::Entity;
    use super::types::SearchSource;
    use super::types::TimelineEvent;
    use super::*;

    #[test]
    fn entity_upsert_is_cas_guarded() {
        let s = MemoryStore::open_in_memory().unwrap();
        let e = Entity {
            id: "guyuan".into(),
            name: "顾远".into(),
            confidence: "canon".into(),
            ..Default::default()
        };
        let v1 = s.upsert_entity(&e, Some(0)).unwrap(); // create at version 0 -> 1
        assert_eq!(v1, 1);
        let got = s.get_entity("guyuan").unwrap().unwrap();
        assert_eq!(got.name, "顾远");
        assert_eq!(got.version, 1);
        let err = s.upsert_entity(&e, Some(0)).unwrap_err(); // stale version rejected
        assert!(matches!(
            err,
            MemoryError::VersionConflict {
                expected: 0,
                actual: 1
            }
        ));
        let v2 = s.upsert_entity(&e, Some(1)).unwrap(); // correct version bumps
        assert_eq!(v2, 2);
    }

    #[test]
    fn timeline_insert_is_idempotent_and_range_sorts_by_diegetic_seq() {
        let s = MemoryStore::open_in_memory().unwrap();
        let ev = |id: &str, seq: i64, sum: &str| TimelineEvent {
            event_id: id.into(),
            diegetic_seq: seq,
            summary: sum.into(),
            ..Default::default()
        };
        assert!(s
            .insert_timeline_event(&ev("e2", 138, "顾远向林安安表白"))
            .unwrap());
        assert!(s.insert_timeline_event(&ev("e1", 100, "序章")).unwrap());
        assert!(
            !s.insert_timeline_event(&ev("e2", 138, "dup")).unwrap(),
            "same event_id is a no-op"
        );
        let range = s.timeline_range(0, 200).unwrap();
        assert_eq!(
            range.iter().map(|e| e.event_id.as_str()).collect::<Vec<_>>(),
            vec!["e1", "e2"]
        );
        assert_eq!(s.timeline_range(120, 200).unwrap().len(), 1);
    }

    #[test]
    fn search_finds_cjk_substring_across_entities_and_timeline() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.upsert_entity(
            &Entity {
                id: "luoyunxi".into(),
                name: "洛云希".into(),
                ..Default::default()
            },
            Some(0),
        )
        .unwrap();
        s.insert_timeline_event(&TimelineEvent {
            event_id: "e1".into(),
            diegetic_seq: 1,
            summary: "顾远向林安安表白，洛云希在场".into(),
            ..Default::default()
        })
        .unwrap();
        // mid-string CJK substring matches the timeline summary
        let hits = s.search("向林安", 10).unwrap();
        assert!(hits
            .iter()
            .any(|h| h.id == "e1" && h.source == SearchSource::Timeline));
        // a name that appears in BOTH an entity and a timeline summary
        let hits2 = s.search("洛云希", 10).unwrap();
        assert!(hits2
            .iter()
            .any(|h| h.source == SearchSource::Entity && h.id == "luoyunxi"));
        assert!(hits2
            .iter()
            .any(|h| h.source == SearchSource::Timeline && h.id == "e1"));
    }

    #[test]
    fn message_index_resolves_and_clear_all_wipes_derived_data() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.set_message_index("m_a", 0, "sha_a").unwrap();
        s.set_message_index("m_b", 1, "sha_b").unwrap();
        assert_eq!(s.position_of_msg("m_b").unwrap(), Some(1));
        s.set_message_index("m_b", 5, "sha_b2").unwrap(); // upsert updates
        assert_eq!(s.position_of_msg("m_b").unwrap(), Some(5));
        s.upsert_entity(&Entity { id: "x".into(), ..Default::default() }, Some(0))
            .unwrap();
        s.set_watermark(5).unwrap();

        s.clear_all().unwrap(); // rebuild: wipe derived data, keep schema, reset watermark
        assert_eq!(s.get_entity("x").unwrap(), None);
        assert_eq!(s.position_of_msg("m_b").unwrap(), None);
        assert_eq!(s.watermark().unwrap(), 0);
        assert_eq!(s.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
        assert!(s.table_exists("entities").unwrap()); // tables remain
    }

    #[test]
    fn open_in_memory_migrates_schema_and_sets_version() {
        let store = MemoryStore::open_in_memory().unwrap();
        for t in [
            "meta",
            "entities",
            "timeline",
            "threads",
            "state",
            "plan",
            "overrides",
            "message_index",
            "entities_fts",
            "timeline_fts",
        ] {
            assert!(store.table_exists(t).unwrap(), "missing table {t}");
        }
        assert_eq!(store.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
        assert_eq!(store.watermark().unwrap(), 0);
    }
}
