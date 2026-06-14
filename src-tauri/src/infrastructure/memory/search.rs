// P2a full-text search: FTS5 trigram MATCH (3+ char terms) with a LIKE
// substring fallback (shorter terms), over entities + timeline.
//
// The query is split on whitespace into terms that are OR-combined, so a
// multi-keyword query like `顾远 白钢城 道具` matches a row containing ANY term
// instead of requiring the whole string to appear contiguously as one phrase
// (the previous behaviour, which silently missed every multi-keyword recall).
//
// Terms of 3+ characters go through the FTS5 trigram index, which matches
// mid-string CJK substrings the default unicode61 tokenizer cannot. Terms
// shorter than 3 characters — e.g. 2-char CJK names like `顾远` — cannot form a
// trigram token and never match through FTS5, so they fall back to a
// `LIKE '%term%'` scan of the source columns. Per-chat memory stores are small,
// so the unindexed LIKE scan is cheap.
use std::collections::HashSet;

use super::types::{SearchHit, SearchSource};
use super::{MemoryResult, MemoryStore};

impl MemoryStore {
    /// Search entity names/aliases and timeline summaries for `query`, returning
    /// at most `limit` hits (entities first, then timeline events).
    ///
    /// `query` is tokenized on whitespace and the terms are OR-combined: a row
    /// matches if it contains any term. Stray double-quotes are stripped so a
    /// term cannot break out of FTS5 phrase syntax. Results are de-duplicated by
    /// `(source, id)` so a row matched by more than one term appears once.
    pub fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchHit>> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| term.replace('"', ""))
            .filter(|term| !term.is_empty())
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        // 3+ char terms -> FTS5 phrases (OR'd); shorter terms -> LIKE fallback.
        let mut fts_phrases: Vec<String> = Vec::new();
        let mut like_terms: Vec<String> = Vec::new();
        for term in &terms {
            if term.chars().count() >= 3 {
                fts_phrases.push(format!("\"{term}\""));
            } else {
                like_terms.push(term.clone());
            }
        }
        let fts_query = fts_phrases.join(" OR ");

        let limit_i64 = limit as i64;
        let conn = self.conn.lock().unwrap();
        let mut out: Vec<SearchHit> = Vec::new();
        let mut seen: HashSet<(u8, String)> = HashSet::new();

        // --- Entities (name + aliases) first, to preserve the ordering contract.
        if !fts_query.is_empty() {
            let mut stmt = conn
                .prepare("SELECT id, name FROM entities_fts WHERE entities_fts MATCH ?1 LIMIT ?2")?;
            let rows = stmt.query_map(rusqlite::params![fts_query, limit_i64], row_to_entity_hit)?;
            push_hits(rows, &mut out, &mut seen)?;
        }
        if !like_terms.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT id, name FROM entities \
                 WHERE name LIKE ?1 ESCAPE '\\' OR aliases LIKE ?1 ESCAPE '\\' LIMIT ?2",
            )?;
            for term in &like_terms {
                let pattern = like_pattern(term);
                let rows =
                    stmt.query_map(rusqlite::params![pattern, limit_i64], row_to_entity_hit)?;
                push_hits(rows, &mut out, &mut seen)?;
            }
        }

        // --- Timeline (summary).
        if !fts_query.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT event_id, summary FROM timeline_fts WHERE timeline_fts MATCH ?1 LIMIT ?2",
            )?;
            let rows =
                stmt.query_map(rusqlite::params![fts_query, limit_i64], row_to_timeline_hit)?;
            push_hits(rows, &mut out, &mut seen)?;
        }
        if !like_terms.is_empty() {
            let mut stmt = conn
                .prepare("SELECT event_id, summary FROM timeline WHERE summary LIKE ?1 ESCAPE '\\' LIMIT ?2")?;
            for term in &like_terms {
                let pattern = like_pattern(term);
                let rows =
                    stmt.query_map(rusqlite::params![pattern, limit_i64], row_to_timeline_hit)?;
                push_hits(rows, &mut out, &mut seen)?;
            }
        }

        out.truncate(limit);
        Ok(out)
    }
}

fn row_to_entity_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        id: row.get::<_, String>(0)?,
        source: SearchSource::Entity,
        snippet: row.get::<_, String>(1)?,
    })
}

fn row_to_timeline_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        id: row.get::<_, String>(0)?,
        source: SearchSource::Timeline,
        snippet: row.get::<_, String>(1)?,
    })
}

/// Push mapped rows into `out`, skipping `(source, id)` pairs already seen so a
/// row matched by both the FTS pass and a LIKE term is not duplicated.
fn push_hits(
    rows: impl Iterator<Item = rusqlite::Result<SearchHit>>,
    out: &mut Vec<SearchHit>,
    seen: &mut HashSet<(u8, String)>,
) -> MemoryResult<()> {
    for hit in rows {
        let hit = hit?;
        let key = (source_discriminant(&hit.source), hit.id.clone());
        if seen.insert(key) {
            out.push(hit);
        }
    }
    Ok(())
}

fn source_discriminant(source: &SearchSource) -> u8 {
    match source {
        SearchSource::Entity => 0,
        SearchSource::Timeline => 1,
    }
}

/// Build a `LIKE` pattern that matches `term` as a substring, escaping the
/// LIKE wildcards (`%`, `_`) and the escape char itself with `\` (paired with
/// `ESCAPE '\\'` in the query) so a term containing them matches literally.
fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}
