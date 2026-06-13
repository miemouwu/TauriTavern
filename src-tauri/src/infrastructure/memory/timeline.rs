// P2a timeline ops: idempotent append + diegetic-ordered range query.
use rusqlite::Row;

use super::types::TimelineEvent;
use super::{MemoryResult, MemoryStore};

impl MemoryStore {
    /// Append a timeline event, keyed by `event_id`.
    ///
    /// Insertion is idempotent: a second insert with an already-present `event_id`
    /// is a no-op (`ON CONFLICT(event_id) DO NOTHING`) and returns `Ok(false)`.
    /// When a new row is written the matching `timeline_fts` row is created and
    /// `Ok(true)` is returned. The JSON-shaped columns (`participants`,
    /// `notable_absent`, `source_refs`) are stored as serialized JSON text.
    pub fn insert_timeline_event(&self, ev: &TimelineEvent) -> MemoryResult<bool> {
        let conn = self.conn.lock().unwrap();

        let participants = serde_json::to_string(&ev.participants)?;
        let notable_absent = serde_json::to_string(&ev.notable_absent)?;
        let source_refs = serde_json::to_string(&ev.source_refs)?;

        let affected = conn.execute(
            "INSERT INTO timeline(event_id,narration_order,diegetic_seq,story_time_label,frame,status,summary,participants,notable_absent,source_refs)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(event_id) DO NOTHING",
            rusqlite::params![
                ev.event_id,
                ev.narration_order,
                ev.diegetic_seq,
                ev.story_time_label,
                ev.frame,
                ev.status,
                ev.summary,
                participants,
                notable_absent,
                source_refs,
            ],
        )?;

        if affected > 0 {
            conn.execute(
                "INSERT INTO timeline_fts(event_id, summary) VALUES(?1,?2)",
                rusqlite::params![ev.event_id, ev.summary],
            )?;
        }

        Ok(affected > 0)
    }

    /// Fetch timeline events whose `diegetic_seq` falls within `[from_seq, to_seq]`
    /// (inclusive), ordered by `diegetic_seq` ascending. JSON-shaped columns are
    /// deserialized back into their typed fields.
    pub fn timeline_range(&self, from_seq: i64, to_seq: i64) -> MemoryResult<Vec<TimelineEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT event_id,narration_order,diegetic_seq,story_time_label,frame,status,summary,participants,notable_absent,source_refs
             FROM timeline WHERE diegetic_seq BETWEEN ?1 AND ?2 ORDER BY diegetic_seq",
        )?;
        let rows = stmt.query_map(rusqlite::params![from_seq, to_seq], Self::row_to_timeline_raw)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(Self::timeline_from_raw(row?)?);
        }
        Ok(out)
    }

    /// Pull the raw column tuple from a timeline row. JSON columns come back as
    /// `String` and are parsed in [`timeline_from_raw`] so serde errors surface as
    /// [`MemoryError::Serde`](super::MemoryError::Serde) rather than rusqlite errors.
    #[allow(clippy::type_complexity)]
    fn row_to_timeline_raw(
        r: &Row<'_>,
    ) -> rusqlite::Result<(String, i64, i64, String, String, String, String, String, String, String)>
    {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
        ))
    }

    #[allow(clippy::type_complexity)]
    fn timeline_from_raw(
        row: (String, i64, i64, String, String, String, String, String, String, String),
    ) -> MemoryResult<TimelineEvent> {
        Ok(TimelineEvent {
            event_id: row.0,
            narration_order: row.1,
            diegetic_seq: row.2,
            story_time_label: row.3,
            frame: row.4,
            status: row.5,
            summary: row.6,
            participants: serde_json::from_str(&row.7)?,
            notable_absent: serde_json::from_str(&row.8)?,
            source_refs: serde_json::from_str(&row.9)?,
        })
    }
}
