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
#[allow(dead_code)] // consumed by P3 consolidation; part of the P0 id↔position resolver
pub fn position_of(messages: &[ChatMessage], msg_id: &str) -> Option<usize> {
    messages.iter().position(|m| {
        m.extra
            .tauritavern
            .as_ref()
            .and_then(|t| t.msg_id.as_deref())
            == Some(msg_id)
    })
}

/// Build a `msgId -> position` index (cache for later consolidation).
#[allow(dead_code)] // consumed by P3 consolidation; part of the P0 id↔position resolver
pub fn build_id_index(messages: &[ChatMessage]) -> HashMap<String, usize> {
    let mut index = HashMap::new();
    for (pos, m) in messages.iter().enumerate() {
        if let Some(id) = m.extra.tauritavern.as_ref().and_then(|t| t.msg_id.clone()) {
            index.insert(id, pos);
        }
    }
    index
}

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
        msgs.insert(0, ChatMessage::user("u", "z"));
        assert_eq!(position_of(&msgs, &id1), Some(2));
        let idx = build_id_index(&msgs);
        assert_eq!(idx.get(&id1), Some(&2));
    }

    #[test]
    fn resolver_handles_delete_shift_and_duplicate_first_match() {
        let mut msgs = vec![
            ChatMessage::user("u", "a"),
            ChatMessage::character("b", "b"),
            ChatMessage::user("u", "c"),
        ];
        stamp_all(&mut msgs);
        let id2 = msgs[2]
            .extra
            .tauritavern
            .as_ref()
            .unwrap()
            .msg_id
            .clone()
            .unwrap();
        assert_eq!(position_of(&msgs, &id2), Some(2));
        msgs.remove(0); // delete shifts positions down
        assert_eq!(position_of(&msgs, &id2), Some(1));
        // duplicate id resolves to the FIRST occurrence
        let dup = msgs[0].clone();
        msgs.push(dup);
        let id0 = msgs[0]
            .extra
            .tauritavern
            .as_ref()
            .unwrap()
            .msg_id
            .clone()
            .unwrap();
        assert_eq!(position_of(&msgs, &id0), Some(0));
    }
}
