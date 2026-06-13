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
