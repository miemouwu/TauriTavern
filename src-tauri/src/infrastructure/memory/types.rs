use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub relationships: Value,
    pub address: Value,
    pub state: Value,
    pub voice: String,
    pub last_confirmed_turn: i64,
    pub confidence: String,
    pub source_refs: Value,
    pub version: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub event_id: String,
    pub narration_order: i64,
    pub diegetic_seq: i64,
    pub story_time_label: String,
    pub frame: String,
    pub status: String,
    pub summary: String,
    pub participants: Vec<String>,
    pub notable_absent: Vec<String>,
    pub source_refs: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchSource {
    Entity,
    Timeline,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: String, // entity id or event_id
    pub source: SearchSource,
    pub snippet: String, // the matched text (name/summary)
}
