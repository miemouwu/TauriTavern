use serde::Serialize;
use serde_json::{Map, Value};

use super::{DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT};
use crate::application::errors::ApplicationError;
use crate::application::services::agent_tools::common::{
    object_args, optional_usize_arg, required_trimmed_string_arg, tool_error,
};
use crate::application::services::agent_tools::structured::structured_value;
use crate::domain::models::agent::{AgentToolCall, AgentToolResult};
use crate::infrastructure::memory::types::{Entity, SearchHit, SearchSource, TimelineEvent};
use crate::infrastructure::memory::{MemoryError, MemoryStore};

use super::super::dispatcher::AgentToolEffect;

#[derive(Serialize)]
struct MemorySearchHitStructured<'a> {
    id: &'a str,
    source: &'a str,
    snippet: &'a str,
}

#[derive(Serialize)]
struct MemorySearchStructured<'a> {
    hits: Vec<MemorySearchHitStructured<'a>>,
}

#[derive(Serialize)]
struct MemoryReadStructured {
    entity: Option<Entity>,
}

#[derive(Serialize)]
struct MemoryTimelineStructured {
    events: Vec<TimelineEvent>,
}

#[derive(Serialize)]
struct MemoryProposeEntityStructured {
    version: i64,
}

#[derive(Serialize)]
struct MemoryProposeEventStructured {
    inserted: bool,
}

fn source_as_str(source: &SearchSource) -> &'static str {
    match source {
        SearchSource::Entity => "entity",
        SearchSource::Timeline => "timeline",
    }
}

/// Map an infrastructural [`MemoryError`] (sqlite/serde) onto a host-level
/// [`ApplicationError`]. The recoverable cases ([`MemoryError::VersionConflict`])
/// are handled at the call site as tool errors and never reach this function.
fn memory_internal_error(error: MemoryError) -> ApplicationError {
    ApplicationError::InternalError(format!("agent.memory_store_error: {error:?}"))
}

/// Deserialize a model-supplied `data` object into a `Default`-able record,
/// filling any omitted fields from the type's default instead of rejecting the
/// write. `Entity`/`TimelineEvent` derive `Deserialize` without per-field
/// defaults, so a strict `from_value` fails the moment the model leaves out a
/// structural field it does not care about (e.g. `aliases`, `version`). The
/// model can only be trusted to send the fields it means to set; the code stays
/// tolerant by overlaying those keys onto the default record's JSON. A field
/// present with the wrong JSON type (e.g. a numeric `id`) still fails, so
/// genuinely malformed input surfaces as a tool error.
fn deserialize_with_defaults<T>(data: &Value) -> Result<T, serde_json::Error>
where
    T: Default + Serialize + serde::de::DeserializeOwned,
{
    let mut merged = serde_json::to_value(T::default())?;
    if let (Some(target), Some(source)) = (merged.as_object_mut(), data.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(merged)
}

/// Read a `from`/`to` style optional signed integer argument used by the
/// timeline range query. Mirrors `optional_usize_arg`'s `Result<Option<_>,
/// String>` shape so callers surface bad arguments as recoverable tool errors.
fn optional_i64_arg(args: &Map<String, Value>, key: &str) -> Result<Option<i64>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_i64()
        .map(Some)
        .ok_or_else(|| format!("{key} must be an integer"))
}

pub(in crate::application::services::agent_tools) async fn search(
    store: &MemoryStore,
    call: &AgentToolCall,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let Some(args) = object_args(call) else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "arguments must be an object"),
            AgentToolEffect::None,
        ));
    };
    let Some(query) = required_trimmed_string_arg(args, "query") else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "query is required"),
            AgentToolEffect::None,
        ));
    };
    let limit = match optional_usize_arg(args, "limit") {
        Ok(limit) => limit.unwrap_or(DEFAULT_SEARCH_LIMIT),
        Err(message) => {
            return Ok((
                tool_error(call, "tool.invalid_arguments", &message),
                AgentToolEffect::None,
            ));
        }
    };
    if limit == 0 {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "limit must be >= 1"),
            AgentToolEffect::None,
        ));
    }
    let limit = limit.min(MAX_SEARCH_LIMIT);

    let hits = store.search(query, limit).map_err(memory_internal_error)?;
    let content = render_search_content(query, &hits);
    let structured = MemorySearchStructured {
        hits: hits
            .iter()
            .map(|hit| MemorySearchHitStructured {
                id: hit.id.as_str(),
                source: source_as_str(&hit.source),
                snippet: hit.snippet.as_str(),
            })
            .collect(),
    };
    let resource_refs = hits.iter().map(|hit| hit.id.clone()).collect();

    Ok((
        AgentToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            structured: structured_value(structured),
            is_error: false,
            error_code: None,
            resource_refs,
        },
        AgentToolEffect::None,
    ))
}

pub(in crate::application::services::agent_tools) async fn read(
    store: &MemoryStore,
    call: &AgentToolCall,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let Some(args) = object_args(call) else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "arguments must be an object"),
            AgentToolEffect::None,
        ));
    };
    let Some(id) = required_trimmed_string_arg(args, "id") else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "id is required"),
            AgentToolEffect::None,
        ));
    };

    let entity = store.get_entity(id).map_err(memory_internal_error)?;
    let content = match &entity {
        Some(entity) => format!(
            "Entity `{}` ({}) at version {}.",
            entity.id, entity.name, entity.version
        ),
        None => format!("No memory entity found for id `{id}`."),
    };

    Ok((
        AgentToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            structured: structured_value(MemoryReadStructured {
                entity: entity.clone(),
            }),
            is_error: false,
            error_code: None,
            resource_refs: entity.map(|entity| vec![entity.id]).unwrap_or_default(),
        },
        AgentToolEffect::None,
    ))
}

pub(in crate::application::services::agent_tools) async fn timeline(
    store: &MemoryStore,
    call: &AgentToolCall,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let Some(args) = object_args(call) else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "arguments must be an object"),
            AgentToolEffect::None,
        ));
    };
    let from = match optional_i64_arg(args, "from") {
        Ok(from) => from.unwrap_or(0),
        Err(message) => {
            return Ok((
                tool_error(call, "tool.invalid_arguments", &message),
                AgentToolEffect::None,
            ));
        }
    };
    let to = match optional_i64_arg(args, "to") {
        Ok(to) => to.unwrap_or(i64::MAX),
        Err(message) => {
            return Ok((
                tool_error(call, "tool.invalid_arguments", &message),
                AgentToolEffect::None,
            ));
        }
    };
    if from > to {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "from must be <= to"),
            AgentToolEffect::None,
        ));
    }

    let events = store
        .timeline_range(from, to)
        .map_err(memory_internal_error)?;
    let content = if events.is_empty() {
        format!("No timeline events in diegetic_seq range [{from}, {to}].")
    } else {
        format!(
            "{} timeline event{} in diegetic_seq range [{from}, {to}].",
            events.len(),
            if events.len() == 1 { "" } else { "s" }
        )
    };
    let resource_refs = events.iter().map(|event| event.event_id.clone()).collect();

    Ok((
        AgentToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            structured: structured_value(MemoryTimelineStructured { events }),
            is_error: false,
            error_code: None,
            resource_refs,
        },
        AgentToolEffect::None,
    ))
}

pub(in crate::application::services::agent_tools) async fn propose(
    store: &MemoryStore,
    call: &AgentToolCall,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let Some(args) = object_args(call) else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "arguments must be an object"),
            AgentToolEffect::None,
        ));
    };
    let Some(kind) = required_trimmed_string_arg(args, "kind") else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "kind is required"),
            AgentToolEffect::None,
        ));
    };
    let Some(data) = args.get("data") else {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "data is required"),
            AgentToolEffect::None,
        ));
    };
    if !data.is_object() {
        return Ok((
            tool_error(call, "tool.invalid_arguments", "data must be an object"),
            AgentToolEffect::None,
        ));
    }

    match kind {
        "entity" => propose_entity(store, call, args, data).await,
        "event" => propose_event(store, call, data).await,
        other => Ok((
            tool_error(
                call,
                "tool.invalid_arguments",
                &format!("kind must be \"entity\" or \"event\", got `{other}`"),
            ),
            AgentToolEffect::None,
        )),
    }
}

async fn propose_entity(
    store: &MemoryStore,
    call: &AgentToolCall,
    args: &Map<String, Value>,
    data: &Value,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let expected_version = match optional_i64_arg(args, "expectedVersion") {
        Ok(expected_version) => expected_version,
        Err(message) => {
            return Ok((
                tool_error(call, "tool.invalid_arguments", &message),
                AgentToolEffect::None,
            ));
        }
    };
    let entity: Entity = match deserialize_with_defaults(data) {
        Ok(entity) => entity,
        Err(error) => {
            return Ok((
                tool_error(
                    call,
                    "tool.invalid_arguments",
                    &format!("data is not a valid entity: {error}"),
                ),
                AgentToolEffect::None,
            ));
        }
    };
    if entity.id.trim().is_empty() {
        return Ok((
            tool_error(
                call,
                "tool.invalid_arguments",
                "data.id is required for an entity",
            ),
            AgentToolEffect::None,
        ));
    }

    match store.upsert_entity(&entity, expected_version) {
        Ok(version) => Ok((
            AgentToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!(
                    "Stored entity `{}` at version {version}.",
                    entity.id.as_str()
                ),
                structured: structured_value(MemoryProposeEntityStructured { version }),
                is_error: false,
                error_code: None,
                resource_refs: vec![entity.id],
            },
            AgentToolEffect::None,
        )),
        Err(MemoryError::VersionConflict { expected, actual }) => Ok((
            tool_error(
                call,
                "memory.version_conflict",
                &format!(
                    "entity `{}` changed since you read it: expected version {expected}, current version is {actual}. Re-read the entity and retry with expectedVersion {actual}.",
                    entity.id.as_str()
                ),
            ),
            AgentToolEffect::None,
        )),
        Err(other) => Err(memory_internal_error(other)),
    }
}

async fn propose_event(
    store: &MemoryStore,
    call: &AgentToolCall,
    data: &Value,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let event: TimelineEvent = match deserialize_with_defaults(data) {
        Ok(event) => event,
        Err(error) => {
            return Ok((
                tool_error(
                    call,
                    "tool.invalid_arguments",
                    &format!("data is not a valid timeline event: {error}"),
                ),
                AgentToolEffect::None,
            ));
        }
    };
    if event.event_id.trim().is_empty() {
        return Ok((
            tool_error(
                call,
                "tool.invalid_arguments",
                "data.event_id is required for a timeline event",
            ),
            AgentToolEffect::None,
        ));
    }

    let inserted = store
        .insert_timeline_event(&event)
        .map_err(memory_internal_error)?;
    let content = if inserted {
        format!("Inserted timeline event `{}`.", event.event_id.as_str())
    } else {
        format!(
            "Timeline event `{}` already exists; insert was a no-op.",
            event.event_id.as_str()
        )
    };

    Ok((
        AgentToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            structured: structured_value(MemoryProposeEventStructured { inserted }),
            is_error: false,
            error_code: None,
            resource_refs: vec![event.event_id],
        },
        AgentToolEffect::None,
    ))
}

fn render_search_content(query: &str, hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return format!("No memory entries matched `{query}`.");
    }
    let mut content = format!(
        "Search `{query}` matched {} memory entr{}. Use memory_read for entities or memory_timeline for events.",
        hits.len(),
        if hits.len() == 1 { "y" } else { "ies" }
    );
    for hit in hits {
        content.push_str(&format!(
            "\n\n{} {} {}",
            source_as_str(&hit.source),
            hit.id,
            hit.snippet
        ));
    }
    content
}
