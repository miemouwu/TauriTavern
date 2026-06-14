# P0 — Message Identity + Token Estimation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every chat message a stable identity (`extra.tauritavern.msgId` + `contentSha`) and expose a per-request token budget — the two prerequisites (P0) for the Agent Memory roadmap.

**Architecture:** Message identity is stamped **in the frontend** at save/load (because the windowed save path sends pre-serialized JSONL lines to the backend, so the backend never sees message objects on the interactive path — see `docs/Agent/MemoryImplementationRoadmap.md` §2.1). The id is a random `uuidv4` assigned once and persisted-on-save, so it is stable across re-saves; `contentSha = sha256(mes)` is recomputed every save. The backend gains a typed `MessageExtra.tauritavern` field (round-trips the data), a Rust `id↔position` resolver (for P3 consolidation), and a one-time bulk-backfill command (for migrated/legacy chats). Token estimation is pure backend: a `model→max_context` registry + a `PromptBudget` computed in the model gateway after the provider payload is encoded.

**Tech Stack:** Rust (`tauritavern` crate, `sha2 0.10`, `uuid 1.4 v4`, `async-trait`, `tokio`), TypeScript/JS frontend (`uuidv4` from `utils.js`, `sha256` from `lib.js`), Node `--test` contract tests.

**Two independent parts** — execute in any order:
- **Part A** — Message Identity (frontend + backend)
- **Part B** — Token Estimation (backend only)

**Conventions:**
- Run Rust tests from `src-tauri`: `source "$HOME/.cargo/env" && cargo test -p tauritavern <name>`
- Run a single contract test: `node --test "tests/<name>.test.mjs"`
- After any `src/` change, before manual app testing: `pnpm run web:build` (no `beforeDevCommand` — bundles are not auto-rebuilt).

---

## File Structure

**Part A — created:**
- `src/scripts/tauritavern/message-identity-core.js` — pure, dependency-injected stamping logic (no heavy imports → unit-testable).
- `src/scripts/tauritavern/message-identity.js` — wires the core with real `uuidv4` + `sha256`; app entrypoint.
- `src-tauri/src/domain/models/chat_identity.rs` — Rust `content_sha`, `stamp_message`/`stamp_all`, `build_id_index`, `position_of`.
- `tests/message-identity-contract.test.mjs` — frontend contract test.

**Part A — modified:**
- `src-tauri/src/domain/models/chat.rs` — add `TauritavernMeta` + `MessageExtra.tauritavern` field.
- `src-tauri/src/domain/models/mod.rs` — declare `pub mod chat_identity;`.
- `src/script.js` — call `stampAllMessages` in `saveChatUnsafe` (≈8357) and `getChatResult` (≈8769).
- `src-tauri/src/application/services/chat_service.rs` — `backfill_chat_identity(...)` service method.
- `src-tauri/src/presentation/commands/...` — `backfill_chat_identity` Tauri command + registration.

**Part B — created:**
- `src-tauri/src/application/services/agent_model_gateway/model_context.rs` — `max_context_for` + `PromptBudget`.

**Part B — modified:**
- `src-tauri/src/application/services/agent_model_gateway/mod.rs` — add tokenizer field; compute + log `PromptBudget` in `generate_with_cancel`.
- `src-tauri/src/app/bootstrap.rs:282` — pass the tokenizer into the gateway constructor.

---

## Part A — Message Identity

### Task A1: Backend `TauritavernMeta` type + `MessageExtra.tauritavern` field

**Files:**
- Modify: `src-tauri/src/domain/models/chat.rs` (add type near `MessageExtra`, ~line 134; add field inside `MessageExtra`, before the `additional` flatten at line 176)
- Test: same file's `#[cfg(test)]` module (or `src-tauri/src/domain/models/chat.rs` inline tests)

- [ ] **Step 1: Write the failing test** (add to a `#[cfg(test)] mod identity_tests` at the bottom of `chat.rs`)

```rust
#[cfg(test)]
mod tauritavern_meta_tests {
    use super::*;

    #[test]
    fn message_extra_round_trips_tauritavern_and_preserves_unknown_keys() {
        let raw = serde_json::json!({
            "api": "openai",
            "tauritavern": { "msgId": "m_abc", "contentSha": "deadbeef" },
            "some_future_field": 42
        });
        let extra: MessageExtra = serde_json::from_value(raw).unwrap();
        let meta = extra.tauritavern.as_ref().expect("tauritavern present");
        assert_eq!(meta.msg_id.as_deref(), Some("m_abc"));
        assert_eq!(meta.content_sha.as_deref(), Some("deadbeef"));
        // unknown key survives via the flatten catch-all
        let back = serde_json::to_value(&extra).unwrap();
        assert_eq!(back["some_future_field"], serde_json::json!(42));
        assert_eq!(back["tauritavern"]["msgId"], serde_json::json!("m_abc"));
    }

    #[test]
    fn message_extra_without_tauritavern_omits_the_key() {
        let extra = MessageExtra::default();
        let back = serde_json::to_value(&extra).unwrap();
        assert!(back.get("tauritavern").is_none(), "absent meta must not serialize");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern tauritavern_meta_tests`
Expected: FAIL — `no field 'tauritavern' on MessageExtra` / `TauritavernMeta` not found.

- [ ] **Step 3: Add the type + field**

Add just above `MessageExtra` (before line 135 `pub struct MessageExtra`) in `chat.rs`:

```rust
/// Stable, TauriTavern-owned message identity (Agent Memory provenance).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TauritavernMeta {
    #[serde(default, rename = "msgId", skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,

    #[serde(default, rename = "contentSha", skip_serializing_if = "Option::is_none")]
    pub content_sha: Option<String>,

    #[serde(default, flatten)]
    pub additional: HashMap<String, serde_json::Value>,
}
```

Add this field inside `MessageExtra`, immediately before the existing `#[serde(default, flatten)] pub additional` (line 176):

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tauritavern: Option<TauritavernMeta>,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern tauritavern_meta_tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
cd /Users/shoulifu/tauritavern
git add src-tauri/src/domain/models/chat.rs
git commit -m "feat(agent-memory): add typed TauritavernMeta to MessageExtra (P0)"
```

---

### Task A2: Backend `chat_identity.rs` — content hash, stamp, id↔position resolver

**Files:**
- Create: `src-tauri/src/domain/models/chat_identity.rs`
- Modify: `src-tauri/src/domain/models/mod.rs` (add `pub mod chat_identity;`)

- [ ] **Step 1: Write the failing test** (create `chat_identity.rs` with only its test module first)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::chat::ChatMessage;

    #[test]
    fn content_sha_is_stable_and_changes_with_content() {
        let a = content_sha("hello");
        assert_eq!(a, content_sha("hello"));
        assert_ne!(a, content_sha("hello!"));
        assert_eq!(a.len(), 64); // sha256 hex
    }

    #[test]
    fn stamp_is_idempotent_for_msgid_and_refreshes_sha() {
        let mut m = ChatMessage::character("bot", "first");
        m.extra.tauritavern = None;
        let n = stamp_all(std::slice::from_mut(&mut m));
        assert_eq!(n, 1);
        let id1 = m.extra.tauritavern.as_ref().unwrap().msg_id.clone();
        assert!(id1.is_some());
        // edit content + re-stamp: msgId stable, contentSha refreshed
        m.mes = "edited".to_string();
        let n2 = stamp_all(std::slice::from_mut(&mut m));
        assert_eq!(n2, 0, "already-stamped message is not counted as new");
        let meta = m.extra.tauritavern.as_ref().unwrap();
        assert_eq!(meta.msg_id, id1, "msgId is stable");
        assert_eq!(meta.content_sha.as_deref(), Some(content_sha("edited").as_str()));
    }

    #[test]
    fn resolver_maps_id_to_position_and_survives_insert_shift() {
        let mut msgs = vec![
            ChatMessage::user("u", "a"),
            ChatMessage::character("b", "b"),
        ];
        stamp_all(&mut msgs);
        let id1 = msgs[1].extra.tauritavern.as_ref().unwrap().msg_id.clone().unwrap();
        assert_eq!(position_of(&msgs, &id1), Some(1));
        // insert at front: position shifts, id still resolves
        msgs.insert(0, ChatMessage::user("u", "z"));
        assert_eq!(position_of(&msgs, &id1), Some(2));
        let idx = build_id_index(&msgs);
        assert_eq!(idx.get(&id1), Some(&2));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern chat_identity`
Expected: FAIL — `content_sha`/`stamp_all`/`position_of`/`build_id_index` not found.

- [ ] **Step 3: Implement** (prepend above the test module in `chat_identity.rs`)

```rust
use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::domain::models::chat::ChatMessage;

/// sha256 hex of the active message body. Matches the frontend `sha256(mes)`.
pub fn content_sha(mes: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(mes.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Stamp one message: assign `msgId` once (idempotent), always refresh `contentSha`.
pub fn stamp_message(message: &mut ChatMessage) {
    let meta = message.extra.tauritavern.get_or_insert_with(Default::default);
    if meta.msg_id.is_none() {
        meta.msg_id = Some(uuid::Uuid::new_v4().to_string());
    }
    meta.content_sha = Some(content_sha(&message.mes));
}

/// Stamp every message lacking an id (covers new + legacy backfill).
/// Returns the count of messages that were newly assigned a `msgId`.
pub fn stamp_all(messages: &mut [ChatMessage]) -> usize {
    let mut newly = 0;
    for m in messages.iter_mut() {
        let had = m
            .extra
            .tauritavern
            .as_ref()
            .and_then(|t| t.msg_id.as_ref())
            .is_some();
        stamp_message(m);
        if !had {
            newly += 1;
        }
    }
    newly
}

/// Resolve a `msgId` to its current 0-based position.
pub fn position_of(messages: &[ChatMessage], msg_id: &str) -> Option<usize> {
    messages.iter().position(|m| {
        m.extra
            .tauritavern
            .as_ref()
            .and_then(|t| t.msg_id.as_deref())
            == Some(msg_id)
    })
}

/// Build a `msgId -> position` index (cache for P3 consolidation).
pub fn build_id_index(messages: &[ChatMessage]) -> HashMap<String, usize> {
    let mut index = HashMap::new();
    for (pos, m) in messages.iter().enumerate() {
        if let Some(id) = m.extra.tauritavern.as_ref().and_then(|t| t.msg_id.clone()) {
            index.insert(id, pos);
        }
    }
    index
}
```

Add to `src-tauri/src/domain/models/mod.rs` (alphabetical with the other `pub mod` lines):

```rust
pub mod chat_identity;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern chat_identity`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/domain/models/chat_identity.rs src-tauri/src/domain/models/mod.rs
git commit -m "feat(agent-memory): add chat_identity (sha, stamp, id-position resolver) (P0)"
```

---

### Task A3: Frontend stamping core (pure, injectable) + wired module

**Files:**
- Create: `src/scripts/tauritavern/message-identity-core.js`
- Create: `src/scripts/tauritavern/message-identity.js`
- Test: `tests/message-identity-contract.test.mjs`

- [ ] **Step 1: Write the failing test** (`tests/message-identity-contract.test.mjs`)

```javascript
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeStamper } from '../src/scripts/tauritavern/message-identity-core.js';

// deterministic stubs
const stamper = makeStamper({
    uuid: () => 'uuid-fixed',
    hash: (s) => `h(${s})`,
});

test('stamps a fresh message with msgId + contentSha', () => {
    const m = { mes: 'hi', extra: {} };
    stamper.stampMessage(m);
    assert.equal(m.extra.tauritavern.msgId, 'uuid-fixed');
    assert.equal(m.extra.tauritavern.contentSha, 'h(hi)');
});

test('msgId is idempotent; contentSha refreshes on edit', () => {
    const m = { mes: 'a', extra: { tauritavern: { msgId: 'keep' } } };
    stamper.stampMessage(m);
    assert.equal(m.extra.tauritavern.msgId, 'keep');
    m.mes = 'b';
    stamper.stampMessage(m);
    assert.equal(m.extra.tauritavern.msgId, 'keep');
    assert.equal(m.extra.tauritavern.contentSha, 'h(b)');
});

test('stampAll backfills only unstamped and returns the new count', () => {
    const msgs = [
        { mes: 'x', extra: { tauritavern: { msgId: 'old' } } },
        { mes: 'y', extra: {} },
        { mes: 'z' }, // no extra at all
    ];
    const n = stamper.stampAll(msgs);
    assert.equal(n, 2);
    assert.equal(msgs[0].extra.tauritavern.msgId, 'old');
    assert.equal(msgs[1].extra.tauritavern.msgId, 'uuid-fixed');
    assert.equal(msgs[2].extra.tauritavern.msgId, 'uuid-fixed');
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test "tests/message-identity-contract.test.mjs"`
Expected: FAIL — cannot find module `message-identity-core.js`.

- [ ] **Step 3: Implement the pure core** (`src/scripts/tauritavern/message-identity-core.js`)

```javascript
// Pure, dependency-injected message-identity stamping. No heavy imports so it is
// unit-testable under `node --test`. App wiring lives in ./message-identity.js.

/**
 * @param {{ uuid: () => string, hash: (s: string) => string }} deps
 */
export function makeStamper({ uuid, hash }) {
    function stampMessage(message) {
        if (!message || typeof message !== 'object') return message;
        if (!message.extra || typeof message.extra !== 'object') message.extra = {};
        const prev = message.extra.tauritavern;
        const meta = prev && typeof prev === 'object' ? prev : {};
        if (!meta.msgId) meta.msgId = uuid();
        meta.contentSha = hash(String(message.mes ?? ''));
        message.extra.tauritavern = meta;
        return message;
    }

    /** Stamp every message lacking an id. Returns count newly stamped. */
    function stampAll(messages) {
        let newly = 0;
        for (const m of messages ?? []) {
            const had = m && m.extra && m.extra.tauritavern && m.extra.tauritavern.msgId;
            stampMessage(m);
            if (!had) newly++;
        }
        return newly;
    }

    return { stampMessage, stampAll };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test "tests/message-identity-contract.test.mjs"`
Expected: PASS (3 tests).

- [ ] **Step 5: Add the wired app module** (`src/scripts/tauritavern/message-identity.js`)

```javascript
// App entrypoint: wires the pure core with TauriTavern's uuid + sha256.
import { uuidv4 } from '../utils.js';
import { sha256 } from '../../lib.js';
import { makeStamper } from './message-identity-core.js';

const stamper = makeStamper({ uuid: uuidv4, hash: (s) => sha256(s) });

export const stampMessage = stamper.stampMessage;
export const stampAllMessages = stamper.stampAll;
```

- [ ] **Step 6: Commit**

```bash
git add src/scripts/tauritavern/message-identity-core.js src/scripts/tauritavern/message-identity.js tests/message-identity-contract.test.mjs
git commit -m "feat(agent-memory): frontend message-identity stamper + contract test (P0)"
```

---

### Task A4: Wire frontend stamping into save + load

**Files:**
- Modify: `src/script.js` — import (top, with other `./scripts/tauritavern/...` imports), `saveChatUnsafe` (≈8357), `getChatResult` (≈8769)

- [ ] **Step 1: Add the import** near the other tauritavern imports at the top of `src/script.js`:

```javascript
import { stampAllMessages } from './scripts/tauritavern/message-identity.js';
```

- [ ] **Step 2: Stamp on save.** In `saveChatUnsafe`, after the `fileName` guards (after line 8375, before the save payload is built), insert:

```javascript
    // Agent Memory P0: stamp stable ids on every persisted message (backfills legacy).
    stampAllMessages(chatData ?? chat);
```

- [ ] **Step 3: Stamp on load (immediate backfill on open).** In `getChatResult` (≈8769), after the loaded messages are placed into the module `chat` array (locate the line that populates `chat` from the load result), insert:

```javascript
    // Agent Memory P0: backfill ids when an (old) chat is opened; persisted on next save.
    stampAllMessages(chat);
```

- [ ] **Step 4: Add an integration contract test** (`tests/message-identity-save-contract.test.mjs`)

```javascript
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeStamper } from '../src/scripts/tauritavern/message-identity-core.js';

// Mirrors the saveChatUnsafe call: stampAllMessages(chatData ?? chat) before serialization.
test('a legacy chat array gains stable ids that survive a re-stamp (re-save)', () => {
    const stamper = makeStamper({ uuid: (() => { let n = 0; return () => `uuid-${n++}`; })(), hash: (s) => `h(${s})` });
    const chat = [{ mes: 'old1', extra: {} }, { mes: 'old2', extra: {} }];
    stamper.stampAll(chat);              // first save
    const ids = chat.map(m => m.extra.tauritavern.msgId);
    assert.deepEqual(ids, ['uuid-0', 'uuid-1']);
    stamper.stampAll(chat);              // second save must NOT reassign
    assert.deepEqual(chat.map(m => m.extra.tauritavern.msgId), ids);
});
```

- [ ] **Step 5: Run tests + checks**

Run: `node --test "tests/message-identity-save-contract.test.mjs"` → Expected: PASS.
Run: `pnpm run check` → Expected: guardrails + tsc + contract tests all pass.

- [ ] **Step 6: Rebuild bundle + commit**

```bash
pnpm run web:build
git add src/script.js tests/message-identity-save-contract.test.mjs
git commit -m "feat(agent-memory): stamp message identity on save + load (P0)"
```

---

### Task A5: Backend bulk-backfill command (for migrated/legacy chats & rebuild)

**Files:**
- Modify: `src-tauri/src/application/services/chat_service.rs` — add `backfill_chat_identity`
- Modify: `src-tauri/src/presentation/commands/` — add a `backfill_chat_identity` Tauri command + register it in the command list (follow the pattern of an existing chat command in this module)
- Test: `chat_service.rs` test module

- [ ] **Step 1: Write the failing test** (in `chat_service.rs` `#[cfg(test)]`, mirroring an existing async test that builds a `ChatService` over an in-memory/temp `ChatRepository`)

```rust
#[tokio::test]
async fn backfill_chat_identity_stamps_all_unstamped_and_persists() {
    let service = build_test_chat_service().await; // existing test helper pattern
    let mut chat = Chat::new("user", "Bot");
    chat.file_name = Some("Bot - test".to_string());
    chat.add_message(ChatMessage::user("user", "hello"));
    chat.add_message(ChatMessage::character("Bot", "hi"));
    service.chat_repository.save(&chat).await.unwrap();

    let stamped = service
        .backfill_chat_identity("Bot", "Bot - test")
        .await
        .unwrap();
    assert_eq!(stamped, 2);

    let reloaded = service.chat_repository.get_chat("Bot", "Bot - test").await.unwrap();
    for m in &reloaded.messages {
        assert!(m.extra.tauritavern.as_ref().and_then(|t| t.msg_id.as_ref()).is_some());
    }
    // idempotent: second run stamps nothing new
    let again = service.backfill_chat_identity("Bot", "Bot - test").await.unwrap();
    assert_eq!(again, 0);
}
```

> If no `build_test_chat_service` helper exists, construct the service the same way the nearest existing `#[tokio::test]` in this file does (use that file's repository test double + temp dir).

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern backfill_chat_identity`
Expected: FAIL — method `backfill_chat_identity` not found.

- [ ] **Step 3: Implement the service method** (add to `impl ChatService`)

```rust
/// One-time backfill: stamp stable ids on all messages of an existing chat and persist.
/// Returns the number of messages newly assigned a `msgId`. Idempotent.
pub async fn backfill_chat_identity(
    &self,
    character_name: &str,
    file_name: &str,
) -> Result<usize, ApplicationError> {
    use crate::domain::models::chat_identity::stamp_all;
    let mut chat = self.chat_repository.get_chat(character_name, file_name).await?;
    let newly = stamp_all(&mut chat.messages);
    if newly > 0 {
        self.chat_repository.save(&chat).await?;
    }
    Ok(newly)
}
```

> Adjust the error type / field name (`self.chat_repository`) to match this file's actual `ChatService` struct.

- [ ] **Step 4: Run test to verify it passes**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern backfill_chat_identity`
Expected: PASS.

- [ ] **Step 5: Add the Tauri command** (in the chat commands module, following an existing command's signature/registration)

```rust
#[tauri::command]
pub async fn backfill_chat_identity(
    state: tauri::State<'_, AppState>,
    character_name: String,
    file_name: String,
) -> Result<usize, String> {
    state
        .chat_service
        .backfill_chat_identity(&character_name, &file_name)
        .await
        .map_err(|e| e.to_string())
}
```

Register it in the `tauri::generate_handler![...]` list (same place the other chat commands are registered). Match the actual `AppState` field name for the chat service.

- [ ] **Step 6: Verify build + commit**

Run: `source "$HOME/.cargo/env" && cargo build` (from `src-tauri`) → Expected: compiles.

```bash
git add src-tauri/src/application/services/chat_service.rs src-tauri/src/presentation/
git commit -m "feat(agent-memory): backfill_chat_identity command for legacy chats (P0)"
```

---

## Part B — Token Estimation

### Task B1: `model_context.rs` — `max_context_for` + `PromptBudget`

**Files:**
- Create: `src-tauri/src/application/services/agent_model_gateway/model_context.rs`
- Modify: `src-tauri/src/application/services/agent_model_gateway/mod.rs` (add `mod model_context;` + re-export `PromptBudget`)

- [ ] **Step 1: Write the failing test** (in `model_context.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_context_known_families_and_fallback() {
        assert_eq!(max_context_for("claude-opus-4-8"), 200_000);
        assert_eq!(max_context_for("deepseek-chat"), 65_536);
        assert_eq!(max_context_for("gemini-2.5-pro"), 1_000_000);
        assert_eq!(max_context_for("gpt-4o-mini"), 128_000);
        assert_eq!(max_context_for("totally-unknown-model"), 8_192); // conservative fallback
    }

    #[test]
    fn prompt_budget_ratio_math() {
        let b = PromptBudget::new(64_000, 128_000);
        assert_eq!(b.tokens, 64_000);
        assert_eq!(b.max_context, 128_000);
        assert!((b.ratio - 0.5).abs() < 1e-9);
        // zero context guarded
        assert_eq!(PromptBudget::new(10, 0).ratio, 0.0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern model_context`
Expected: FAIL — module/symbols not found.

- [ ] **Step 3: Implement** (prepend in `model_context.rs`)

```rust
/// Maximum context window (tokens) for a model id. Substring match on the model
/// family, with a conservative fallback. Tunable (roadmap §7 lists exact values as open).
pub fn max_context_for(model: &str) -> usize {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        200_000
    } else if m.contains("deepseek") {
        65_536
    } else if m.contains("gemini") {
        1_000_000
    } else if m.contains("gpt-4o") || m.contains("gpt-4.1") || m.contains("gpt-4-turbo") {
        128_000
    } else if m.contains("gemma") {
        8_192
    } else {
        8_192
    }
}

/// Encoded-payload token budget for one model request.
#[derive(Debug, Clone, Copy)]
pub struct PromptBudget {
    pub tokens: usize,
    pub max_context: usize,
    pub ratio: f64,
}

impl PromptBudget {
    pub fn new(tokens: usize, max_context: usize) -> Self {
        let ratio = if max_context == 0 {
            0.0
        } else {
            tokens as f64 / max_context as f64
        };
        Self { tokens, max_context, ratio }
    }
}
```

Add to `mod.rs` (with the other `mod` declarations near the top): `mod model_context;` and (with the other `pub use`) `pub use model_context::{max_context_for, PromptBudget};`.

- [ ] **Step 4: Run test to verify it passes**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern model_context`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/application/services/agent_model_gateway/model_context.rs src-tauri/src/application/services/agent_model_gateway/mod.rs
git commit -m "feat(agent-memory): model max-context registry + PromptBudget (P0)"
```

---

### Task B2: Wire the tokenizer into the gateway + compute/log `PromptBudget`

**Files:**
- Modify: `src-tauri/src/application/services/agent_model_gateway/mod.rs` (`ChatCompletionAgentModelGateway` struct + `new` at line 41–51; `generate_with_cancel` at 55–75)
- Modify: `src-tauri/src/app/bootstrap.rs:282` (constructor call)
- Test: `src-tauri/src/application/services/agent_model_gateway/tests.rs`

- [ ] **Step 1: Write the failing test** (in `agent_model_gateway/tests.rs`, mirroring its existing style)

```rust
#[test]
fn prompt_budget_from_encoded_payload_uses_count_messages_and_registry() {
    // messages payload as produced by encode_chat_completion_request
    let messages = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": "hi"}),
    ];
    let tokens = 7usize; // pretend the tokenizer counted 7
    let budget = super::model_context::PromptBudget::new(
        tokens,
        super::model_context::max_context_for("deepseek-chat"),
    );
    assert_eq!(budget.max_context, 65_536);
    assert!(budget.ratio > 0.0 && budget.ratio < 0.001);
    assert_eq!(messages.len(), 2);
}
```

> This locks the math + registry wiring used by the gateway. (A full end-to-end gateway test would require the existing `MockAgentModelGateway`/tokenizer doubles in `tests.rs`; if a `TokenizerRepository` mock already exists there, extend it to assert `count_messages` is called with the encoded `messages` array.)

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern prompt_budget_from_encoded_payload`
Expected: FAIL — until `model_context` is reachable from `tests.rs` (it is after B1) the symbol path resolves; if the test references a not-yet-added gateway field it fails to compile.

- [ ] **Step 3: Add the tokenizer to the gateway** — replace the struct + `new` (mod.rs:41–51):

```rust
pub struct ChatCompletionAgentModelGateway {
    chat_completion_service: Arc<ChatCompletionService>,
    tokenizer: Arc<dyn crate::domain::repositories::tokenizer_repository::TokenizerRepository>,
}

impl ChatCompletionAgentModelGateway {
    pub fn new(
        chat_completion_service: Arc<ChatCompletionService>,
        tokenizer: Arc<dyn crate::domain::repositories::tokenizer_repository::TokenizerRepository>,
    ) -> Self {
        Self { chat_completion_service, tokenizer }
    }
}
```

- [ ] **Step 4: Compute + log the budget** — in `generate_with_cancel`, immediately after `let dto = encode::encode_chat_completion_request(&request)?;` (mod.rs:60):

```rust
        // Agent Memory P0: measure the encoded provider payload against the model's window.
        if let (Some(model), Some(messages)) = (
            dto.payload.get("model").and_then(|v| v.as_str()),
            dto.payload.get("messages").and_then(|v| v.as_array()),
        ) {
            if let Ok(tokens) = self.tokenizer.count_messages(model, messages) {
                let budget = model_context::PromptBudget::new(tokens, model_context::max_context_for(model));
                logger::debug(&format!(
                    "agent prompt budget: model={} tokens={} max={} ratio={:.3}",
                    model, budget.tokens, budget.max_context, budget.ratio
                ));
            }
        }
```

> Add `use crate::infrastructure::logger;` (or this file's existing logger import) if not present, and `use crate::application::services::agent_model_gateway::model_context;` (or `super::model_context`).

- [ ] **Step 5: Update the bootstrap constructor** — at `bootstrap.rs:282`, pass the tokenizer. The `TokenizerRepository` is already constructed in bootstrap (it backs `TokenizationService`). Reuse that `Arc`:

```rust
        Arc::new(ChatCompletionAgentModelGateway::new(
            Arc::clone(&chat_completion_service),
            Arc::clone(&tokenizer_repository),
        )),
```

> Use the actual local variable names in `bootstrap.rs` for the chat completion service and the tokenizer `Arc<dyn TokenizerRepository>` (grep `bootstrap.rs` for `TokenizerRepository` / `TokenizationService::new` to find the binding).

- [ ] **Step 6: Run test + build**

Run: `source "$HOME/.cargo/env" && cargo test -p tauritavern prompt_budget_from_encoded_payload` → Expected: PASS.
Run: `source "$HOME/.cargo/env" && cargo build` (from `src-tauri`) → Expected: compiles (gateway constructor updated everywhere it is called — fix any other call sites the compiler flags, e.g. test doubles).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/application/services/agent_model_gateway/ src-tauri/src/app/bootstrap.rs
git commit -m "feat(agent-memory): compute encoded-payload PromptBudget in model gateway (P0)"
```

---

## Self-Review

**Spec coverage (roadmap §2):**
- §2.1 message identity (`extra.tauritavern.msgId` + `contentSha`) → A1 (type), A3/A4 (frontend stamp on create+load), A2 (`content_sha`), A5 (legacy backfill). ✓
- §2.1 `id↔position` resolver → A2 (`position_of`, `build_id_index`). ✓
- §2.1 decision ③ "source_refs only reference already-saved/stamped messages" → satisfied by stamping at save (ids exist on every persisted message) — enforced in P3 when refs are written. ✓ (no P0 code beyond stamping)
- §2.2 token estimation: registry → B1; encoded-payload `count_messages` wired into gateway + ratio → B2. ✓

**Placeholder scan:** Integration call sites in A4/A5/B2 reference exact files/lines and show the exact code to insert; the only soft spots are "match the actual struct/var name" notes where the surrounding identifiers must be read at edit time (unavoidable for in-situ edits). No TBDs, no "handle edge cases", every code step shows code.

**Type consistency:** `TauritavernMeta { msg_id, content_sha }` (A1) used identically by `chat_identity.rs` (A2) and `backfill_chat_identity` (A5). Frontend `tauritavern { msgId, contentSha }` matches the serde renames in A1. `PromptBudget::new(tokens, max_context)` signature identical in B1 + B2.

**Note on `web:build`:** Part A frontend changes require `pnpm run web:build` before manual app testing (Task A4 Step 6); contract tests run against source and do not need it.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-13-p0-message-identity-token-estimation.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
