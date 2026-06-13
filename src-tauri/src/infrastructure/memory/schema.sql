CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT) STRICT;
CREATE TABLE IF NOT EXISTS entities (
  id TEXT PRIMARY KEY, name TEXT, aliases TEXT, relationships TEXT, address TEXT,
  state TEXT, voice TEXT, last_confirmed_turn INTEGER, confidence TEXT,
  source_refs TEXT, version INTEGER NOT NULL DEFAULT 0) STRICT;
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
