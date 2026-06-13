use serde_json::json;

use super::{MEMORY_PROPOSE, MEMORY_READ, MEMORY_SEARCH, MEMORY_TIMELINE};
use crate::domain::models::agent::AgentToolSpec;

const MODEL_MEMORY_SEARCH: &str = "memory_search";
const MODEL_MEMORY_READ: &str = "memory_read";
const MODEL_MEMORY_TIMELINE: &str = "memory_timeline";
const MODEL_MEMORY_PROPOSE: &str = "memory_propose";

pub(in crate::application::services::agent_tools) fn memory_search_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: MEMORY_SEARCH.to_string(),
        model_name: MODEL_MEMORY_SEARCH.to_string(),
        title: "Memory Search".to_string(),
        description: "Search story memory for entities (characters, places) and timeline events whose name, aliases, or summary contain the query text. Substring matches work, including mid-word CJK. Use memory_read to load a matched entity in full, or memory_timeline to read matched events.".to_string(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Plain text to search for across entity names/aliases and timeline summaries."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum hits to return. Defaults to 20; maximum is 100.",
                    "minimum": 1,
                    "maximum": 100
                }
            },
            "required": ["query"]
        }),
        output_schema: None,
        annotations: json!({ "readOnly": true }),
        source: "builtin".to_string(),
    }
}

pub(in crate::application::services::agent_tools) fn memory_read_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: MEMORY_READ.to_string(),
        model_name: MODEL_MEMORY_READ.to_string(),
        title: "Memory Read".to_string(),
        description: "Read a single story-memory entity by its id, including its current version. Use the id returned by memory_search. The structured `entity` field is null when no entity has that id.".to_string(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The entity id to read (as returned by memory_search)."
                }
            },
            "required": ["id"]
        }),
        output_schema: None,
        annotations: json!({ "readOnly": true }),
        source: "builtin".to_string(),
    }
}

pub(in crate::application::services::agent_tools) fn memory_timeline_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: MEMORY_TIMELINE.to_string(),
        model_name: MODEL_MEMORY_TIMELINE.to_string(),
        title: "Memory Timeline".to_string(),
        description: "List story timeline events ordered by their in-story (diegetic) sequence. Filter to a diegetic_seq range with from/to; omit both to read the whole timeline.".to_string(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "from": {
                    "type": "integer",
                    "description": "Inclusive lower bound on diegetic_seq. Defaults to 0."
                },
                "to": {
                    "type": "integer",
                    "description": "Inclusive upper bound on diegetic_seq. Defaults to the maximum, so the rest of the timeline is returned."
                }
            }
        }),
        output_schema: None,
        annotations: json!({ "readOnly": true }),
        source: "builtin".to_string(),
    }
}

pub(in crate::application::services::agent_tools) fn memory_propose_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: MEMORY_PROPOSE.to_string(),
        model_name: MODEL_MEMORY_PROPOSE.to_string(),
        title: "Memory Propose".to_string(),
        description: "Write to story memory. With kind `entity`, upsert an entity from `data`; pass expectedVersion (the version you last read, 0 for a brand-new entity) so a concurrent edit is rejected instead of silently overwritten — on a version conflict, re-read the entity and retry. With kind `event`, append a timeline event from `data`; appends are idempotent on event_id (a duplicate id is a no-op and reports inserted=false).".to_string(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["entity", "event"],
                    "description": "`entity` upserts an entity; `event` appends a timeline event."
                },
                "data": {
                    "type": "object",
                    "description": "The entity object (for kind entity) or timeline event object (for kind event) to store."
                },
                "expectedVersion": {
                    "type": "integer",
                    "description": "For kind entity only: the version you last read for this entity (0 for a new entity). The write is rejected if the stored version differs."
                }
            },
            "required": ["kind", "data"]
        }),
        output_schema: None,
        annotations: json!({ "mutating": true }),
        source: "builtin".to_string(),
    }
}
