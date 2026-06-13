// P2a entity CRUD: upsert (entry-level CAS) + get.
use rusqlite::OptionalExtension;

use super::types::Entity;
use super::{MemoryError, MemoryResult, MemoryStore};

impl MemoryStore {
    /// Upsert an entity with entry-level compare-and-swap.
    ///
    /// `expected_version`, when `Some(v)`, must equal the row's current version
    /// (absent row counts as version `0`); otherwise a [`MemoryError::VersionConflict`]
    /// is returned and nothing is written. On success the stored version is bumped to
    /// `current + 1` and returned. The matching `entities_fts` row is kept in sync.
    pub fn upsert_entity(&self, e: &Entity, expected_version: Option<i64>) -> MemoryResult<i64> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        let current: i64 = tx
            .query_row(
                "SELECT version FROM entities WHERE id=?1",
                rusqlite::params![e.id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);

        if let Some(v) = expected_version {
            if v != current {
                return Err(MemoryError::VersionConflict {
                    expected: v,
                    actual: current,
                });
            }
        }

        let new_version = current + 1;
        let aliases = serde_json::to_string(&e.aliases)?;
        let relationships = serde_json::to_string(&e.relationships)?;
        let address = serde_json::to_string(&e.address)?;
        let state = serde_json::to_string(&e.state)?;
        let source_refs = serde_json::to_string(&e.source_refs)?;

        tx.execute(
            "INSERT INTO entities(id,name,aliases,relationships,address,state,voice,last_confirmed_turn,confidence,source_refs,version)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name,
               aliases=excluded.aliases,
               relationships=excluded.relationships,
               address=excluded.address,
               state=excluded.state,
               voice=excluded.voice,
               last_confirmed_turn=excluded.last_confirmed_turn,
               confidence=excluded.confidence,
               source_refs=excluded.source_refs,
               version=?11",
            rusqlite::params![
                e.id,
                e.name,
                aliases,
                relationships,
                address,
                state,
                e.voice,
                e.last_confirmed_turn,
                e.confidence,
                source_refs,
                new_version,
            ],
        )?;

        // Keep the FTS shadow table in sync. Entities carry no summary; the column
        // exists only for table-shape uniformity, so we store an empty string.
        tx.execute(
            "DELETE FROM entities_fts WHERE id=?1",
            rusqlite::params![e.id],
        )?;
        tx.execute(
            "INSERT INTO entities_fts(id,name,aliases,summary) VALUES(?1,?2,?3,?4)",
            rusqlite::params![e.id, e.name, e.aliases.join(" "), ""],
        )?;

        tx.commit()?;
        Ok(new_version)
    }

    /// Fetch an entity by id, deserializing its JSON-shaped columns.
    pub fn get_entity(&self, id: &str) -> MemoryResult<Option<Entity>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id,name,aliases,relationships,address,state,voice,last_confirmed_turn,confidence,source_refs,version
             FROM entities WHERE id=?1",
            rusqlite::params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, i64>(10)?,
                ))
            },
        )
        .optional()?
        .map(|row| {
            Ok(Entity {
                id: row.0,
                name: row.1,
                aliases: serde_json::from_str(&row.2)?,
                relationships: serde_json::from_str(&row.3)?,
                address: serde_json::from_str(&row.4)?,
                state: serde_json::from_str(&row.5)?,
                voice: row.6,
                last_confirmed_turn: row.7,
                confidence: row.8,
                source_refs: serde_json::from_str(&row.9)?,
                version: row.10,
            })
        })
        .transpose()
    }
}
