use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{MemoryResult, MemoryStore};

/// Opens + caches one MemoryStore per stable_chat_id, under `base_dir/<chat>/memory.db`.
/// base_dir must be OUTSIDE the per-run workspace copy (B-hybrid; also keeps it model-unwritable).
pub struct MemoryStoreProvider {
    base_dir: PathBuf,
    cache: Mutex<HashMap<String, Arc<MemoryStore>>>,
}

impl MemoryStoreProvider {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir, cache: Mutex::new(HashMap::new()) }
    }

    pub async fn get_or_open(&self, stable_chat_id: &str) -> MemoryResult<Arc<MemoryStore>> {
        if let Some(store) = self.cache.lock().unwrap().get(stable_chat_id).cloned() {
            return Ok(store);
        }
        let dir = self.base_dir.join(stable_chat_id);
        let db_path = dir.join("memory.db");
        let store = tokio::task::spawn_blocking(move || -> MemoryResult<MemoryStore> {
            let _ = std::fs::create_dir_all(&dir);
            MemoryStore::open(&db_path)
        })
        .await
        .expect("spawn_blocking join")?;
        let arc = Arc::new(store);
        let mut cache = self.cache.lock().unwrap();
        Ok(cache.entry(stable_chat_id.to_string()).or_insert(arc).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn provider_opens_one_store_per_chat_at_expected_path() {
        let tmp = std::env::temp_dir().join(format!("mem-prov-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let p = MemoryStoreProvider::new(tmp.clone());
        let a = p.get_or_open("chat_1").await.unwrap();
        let b = p.get_or_open("chat_1").await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b), "same chat returns the cached store");
        let c = p.get_or_open("chat_2").await.unwrap();
        assert!(!std::sync::Arc::ptr_eq(&a, &c));
        assert!(tmp.join("chat_1").join("memory.db").exists(), "db at <base>/<chat>/memory.db");
        a.set_watermark(7).unwrap();
        assert_eq!(p.get_or_open("chat_1").await.unwrap().watermark().unwrap(), 7);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
