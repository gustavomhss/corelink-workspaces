use super::required_hit::run_memoized_required_hit;
use super::{compute_key, RunOptions, RunRecord};
use async_trait::async_trait;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Default)]
struct FakeStore {
    ac: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    cas: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    unavailable: bool,
}

#[async_trait]
impl CasTransport for FakeStore {
    async fn get(&self, digest: &Digest) -> Result<Vec<u8>> {
        if self.unavailable {
            return Err(ClwError::Transport("backend unavailable".into()));
        }
        self.cas
            .lock()
            .unwrap()
            .get(&digest.to_hex())
            .cloned()
            .ok_or_else(|| ClwError::NotFound(*digest))
    }
    async fn put(&self, digest: &Digest, bytes: Vec<u8>) -> Result<()> {
        self.cas.lock().unwrap().insert(digest.to_hex(), bytes);
        Ok(())
    }
    async fn exists(&self, digest: &Digest) -> Result<bool> {
        Ok(self.cas.lock().unwrap().contains_key(&digest.to_hex()))
    }
    async fn delete(&self, digest: &Digest) -> Result<()> {
        self.cas.lock().unwrap().remove(&digest.to_hex());
        Ok(())
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        self.cas
            .lock()
            .unwrap()
            .keys()
            .map(|key| Digest::from_hex(key))
            .collect()
    }
}

#[async_trait]
impl AcTransport for FakeStore {
    async fn get(&self, key: &Digest) -> Result<Option<Vec<u8>>> {
        if self.unavailable {
            return Err(ClwError::Transport("backend unavailable".into()));
        }
        Ok(self.ac.lock().unwrap().get(&key.to_hex()).cloned())
    }
    async fn put(&self, key: &Digest, value: Vec<u8>) -> Result<()> {
        self.ac.lock().unwrap().insert(key.to_hex(), value);
        Ok(())
    }
    async fn delete(&self, key: &Digest) -> Result<()> {
        self.ac.lock().unwrap().remove(&key.to_hex());
        Ok(())
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        self.ac
            .lock()
            .unwrap()
            .keys()
            .map(|key| Digest::from_hex(key))
            .collect()
    }
}

fn opts() -> RunOptions {
    RunOptions {
        inputs: vec!["in.txt".into()],
        command: vec!["/bin/true".into()],
        env_keys: vec![],
    }
}

async fn seed_record(tmp: &TempDir, store: &FakeStore, stdout: &[u8], stderr: &[u8]) {
    let options = opts();
    let key = compute_key(tmp.path(), &options).await.unwrap();
    let out = Digest::of_bytes(stdout);
    let err = Digest::of_bytes(stderr);
    AcTransport::put(
        store,
        &key,
        serde_json::to_vec(&RunRecord {
            exit_code: 0,
            stdout: out,
            stderr: err,
        })
        .unwrap(),
    )
    .await
    .unwrap();
    CasTransport::put(store, &out, stdout.to_vec())
        .await
        .unwrap();
    CasTransport::put(store, &err, stderr.to_vec())
        .await
        .unwrap();
}

fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("in.txt"), b"key input").unwrap();
    tmp
}

#[tokio::test]
async fn verified_hit_returns_cached_result() {
    let tmp = fixture();
    let store = FakeStore::default();
    seed_record(&tmp, &store, b"cached", b"").await;
    let hit = run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .unwrap()
        .unwrap();
    assert!(hit.hit);
    assert_eq!(hit.stdout, b"cached");
}

#[tokio::test]
async fn miss_refuses_without_execution_path() {
    let tmp = fixture();
    assert!(
        run_memoized_required_hit(tmp.path(), &FakeStore::default(), &opts())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn corrupt_record_refuses() {
    let tmp = fixture();
    let store = FakeStore::default();
    let key = compute_key(tmp.path(), &opts()).await.unwrap();
    AcTransport::put(&store, &key, b"{".to_vec()).await.unwrap();
    assert!(run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn incomplete_record_refuses() {
    let tmp = fixture();
    let store = FakeStore::default();
    let key = compute_key(tmp.path(), &opts()).await.unwrap();
    AcTransport::put(&store, &key, br#"{"exit_code":0}"#.to_vec())
        .await
        .unwrap();
    assert!(run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn missing_blob_refuses() {
    let tmp = fixture();
    let store = FakeStore::default();
    let key = compute_key(tmp.path(), &opts()).await.unwrap();
    let missing = Digest::of_bytes(b"absent");
    let empty = Digest::of_bytes(b"");
    AcTransport::put(
        &store,
        &key,
        serde_json::to_vec(&RunRecord {
            exit_code: 0,
            stdout: missing,
            stderr: empty,
        })
        .unwrap(),
    )
    .await
    .unwrap();
    CasTransport::put(&store, &empty, vec![]).await.unwrap();
    assert!(run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn backend_failure_is_an_error_not_a_hit() {
    let tmp = fixture();
    let store = FakeStore {
        unavailable: true,
        ..FakeStore::default()
    };
    assert!(run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .is_err());
}

#[tokio::test]
async fn wrong_blob_digest_refuses() {
    let tmp = fixture();
    let store = FakeStore::default();
    let key = compute_key(tmp.path(), &opts()).await.unwrap();
    let expected = Digest::of_bytes(b"expected");
    let empty = Digest::of_bytes(b"");
    AcTransport::put(
        &store,
        &key,
        serde_json::to_vec(&RunRecord {
            exit_code: 0,
            stdout: expected,
            stderr: empty,
        })
        .unwrap(),
    )
    .await
    .unwrap();
    CasTransport::put(&store, &expected, b"wrong".to_vec())
        .await
        .unwrap();
    CasTransport::put(&store, &empty, vec![]).await.unwrap();
    assert!(run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn wrong_stderr_blob_digest_refuses() {
    let tmp = fixture();
    let store = FakeStore::default();
    let key = compute_key(tmp.path(), &opts()).await.unwrap();
    let stdout = b"valid stdout";
    let stdout_digest = Digest::of_bytes(stdout);
    let stderr_digest = Digest::of_bytes(b"expected stderr");
    AcTransport::put(
        &store,
        &key,
        serde_json::to_vec(&RunRecord {
            exit_code: 0,
            stdout: stdout_digest,
            stderr: stderr_digest,
        })
        .unwrap(),
    )
    .await
    .unwrap();
    CasTransport::put(&store, &stdout_digest, stdout.to_vec())
        .await
        .unwrap();
    CasTransport::put(&store, &stderr_digest, b"corrupt stderr".to_vec())
        .await
        .unwrap();

    assert!(run_memoized_required_hit(tmp.path(), &store, &opts())
        .await
        .unwrap()
        .is_none());
}
