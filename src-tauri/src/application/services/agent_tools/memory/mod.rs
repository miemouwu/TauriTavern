// Handlers + name constants are exercised by unit tests and registered specs;
// the dispatcher arms that route `memory.*` calls to them land in P2b Task 3,
// so the handler fns/constants read as dead until then. Mirrors the
// `#![allow(dead_code)]` on the P2a `infrastructure::memory` module. The handler
// re-exports below are consumed by the dispatcher wired in Task 3.
#![allow(dead_code)]
#![allow(unused_imports)]
//! `memory.*` Agent tools: read-only search/read/timeline plus a guarded
//! propose write that operate on the run's [`MemoryStore`]. Each handler mirrors
//! the workspace handlers: it parses `call.arguments`, runs the store op, and
//! either returns a typed structured result or a recoverable model-facing
//! [`tool_error`]. Infrastructural [`MemoryError`]s (sqlite/serde) bubble up as
//! `agent.internal_error`; the recoverable cases (bad arguments, version
//! conflicts) come back as tool errors so the model can self-correct.
mod handlers;
mod specs;

#[cfg(test)]
mod tests;

pub(super) use self::handlers::{propose, read, search, timeline};
pub(super) use self::specs::{
    memory_propose_spec, memory_read_spec, memory_search_spec, memory_timeline_spec,
};

pub(super) const MEMORY_SEARCH: &str = "memory.search";
pub(super) const MEMORY_READ: &str = "memory.read";
pub(super) const MEMORY_TIMELINE: &str = "memory.timeline";
pub(super) const MEMORY_PROPOSE: &str = "memory.propose";

/// Default `limit` for [`MEMORY_SEARCH`] when the model omits it.
const DEFAULT_SEARCH_LIMIT: usize = 20;
/// Cap on `limit` so a single search cannot ask for an unbounded result set.
const MAX_SEARCH_LIMIT: usize = 100;
