// P2a message_index (id<->position resolver) + clear_all rebuild support.
use super::{MemoryResult, MemoryStore};

impl MemoryStore {
    /// Upsert a message's `position` and `content_sha`, keyed by `msg_id`.
    ///
    /// This persists P0's id<->position resolver: a second call with an
    /// already-present `msg_id` updates `position` and `content_sha` in place
    /// (`ON CONFLICT(msg_id) DO UPDATE`).
    pub fn set_message_index(
        &self,
        msg_id: &str,
        position: i64,
        content_sha: &str,
    ) -> MemoryResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO message_index(msg_id,position,content_sha) VALUES(?1,?2,?3)
             ON CONFLICT(msg_id) DO UPDATE SET position=?2, content_sha=?3",
            rusqlite::params![msg_id, position, content_sha],
        )?;
        Ok(())
    }

    /// Resolve a `msg_id` to its stored `position`, or `Ok(None)` if absent.
    pub fn position_of_msg(&self, msg_id: &str) -> MemoryResult<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT position FROM message_index WHERE msg_id=?1",
                rusqlite::params![msg_id],
                |r| r.get::<_, i64>(0),
            )
            .ok())
    }

    /// Wipe all derived data so a fresh rebuild can repopulate it.
    ///
    /// Deletes every data + FTS row in a single transaction and resets the
    /// `watermark` to `0`. The schema itself is left intact: tables are *not*
    /// dropped and `schema_version` is *not* touched, so the store stays usable
    /// immediately after the call.
    pub fn clear_all(&self) -> MemoryResult<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for table in [
            "entities",
            "entities_fts",
            "timeline",
            "timeline_fts",
            "threads",
            "state",
            "plan",
            "overrides",
            "message_index",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute("UPDATE meta SET v='0' WHERE k='watermark'", [])?;
        tx.commit()?;
        Ok(())
    }
}
