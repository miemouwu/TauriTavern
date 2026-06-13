// P2a full-text search: FTS5 trigram MATCH over entities + timeline.
//
// The FTS5 trigram tokenizer indexes every 3-character window, so a quoted
// phrase MATCH does substring search — including mid-string CJK substrings,
// which the default unicode61 tokenizer cannot do for unspaced scripts. The
// query is wrapped in a single FTS5 phrase (`"..."`) so multi-token input is
// matched contiguously rather than as separate OR'd terms.
use super::types::{SearchHit, SearchSource};
use super::{MemoryResult, MemoryStore};

impl MemoryStore {
    /// Search entity names/aliases/summaries and timeline summaries for `query`,
    /// returning at most `limit` hits (entities first, then timeline events).
    ///
    /// `query` is treated as a single FTS5 phrase; stray double-quotes are
    /// stripped so they cannot break out of the phrase syntax. With the trigram
    /// tokenizer the phrase matches as a substring, so a query like `"向林安"`
    /// finds it in the middle of a longer summary.
    pub fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchHit>> {
        let phrase = format!("\"{}\"", query.replace('"', ""));
        let limit_i64 = limit as i64;
        let conn = self.conn.lock().unwrap();

        let mut out = Vec::new();

        let mut entity_stmt = conn.prepare(
            "SELECT id, name FROM entities_fts WHERE entities_fts MATCH ?1 LIMIT ?2",
        )?;
        let entity_rows = entity_stmt.query_map(rusqlite::params![phrase, limit_i64], |r| {
            Ok(SearchHit {
                id: r.get::<_, String>(0)?,
                source: SearchSource::Entity,
                snippet: r.get::<_, String>(1)?,
            })
        })?;
        for hit in entity_rows {
            out.push(hit?);
        }

        let mut timeline_stmt = conn.prepare(
            "SELECT event_id, summary FROM timeline_fts WHERE timeline_fts MATCH ?1 LIMIT ?2",
        )?;
        let timeline_rows = timeline_stmt.query_map(rusqlite::params![phrase, limit_i64], |r| {
            Ok(SearchHit {
                id: r.get::<_, String>(0)?,
                source: SearchSource::Timeline,
                snippet: r.get::<_, String>(1)?,
            })
        })?;
        for hit in timeline_rows {
            out.push(hit?);
        }

        out.truncate(limit);
        Ok(out)
    }
}
