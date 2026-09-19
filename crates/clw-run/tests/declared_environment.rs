//! #302: actual run_memoized, real OS environments, and an isolated child.
//! The parent never changes its process-global environment. Each worker receives
//! one value at spawn and persists only synthetic CAS/AC state between calls.
#![cfg(unix)]

use clw_run::{run_memoized, RunOptions};
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap, ffi::OsString, os::unix::ffi::OsStringExt, path::Path, process::Command,
    sync::Mutex,
};
use tempfile::TempDir;

const VAR: &str = "CLW_302_SYNTHETIC_VALUE";
const SCRIPT: &str = "printf x >> executions; if [ \"${CLW_302_SYNTHETIC_VALUE+x}\" = x ]; then printf '<%s>' \"$CLW_302_SYNTHETIC_VALUE\"; else printf unset; fi";
fn command() -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), SCRIPT.into()]
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    cas: BTreeMap<String, Vec<u8>>,
    ac: BTreeMap<String, Vec<u8>>,
    calls: usize,
}
struct Store(Mutex<State>);
#[async_trait::async_trait]
impl CasTransport for Store {
    async fn exists(&self, d: &Digest) -> Result<bool> {
        let mut s = self.0.lock().unwrap();
        s.calls += 1;
        Ok(s.cas.contains_key(&d.to_hex()))
    }
    async fn get(&self, d: &Digest) -> Result<Vec<u8>> {
        let mut s = self.0.lock().unwrap();
        s.calls += 1;
        s.cas
            .get(&d.to_hex())
            .cloned()
            .ok_or(ClwError::NotFound(*d))
    }
    async fn put(&self, d: &Digest, v: Vec<u8>) -> Result<()> {
        let mut s = self.0.lock().unwrap();
        s.calls += 1;
        s.cas.insert(d.to_hex(), v);
        Ok(())
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("no deletion is authorized by a run")
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        panic!("run does not enumerate storage")
    }
}
#[async_trait::async_trait]
impl AcTransport for Store {
    async fn get(&self, d: &Digest) -> Result<Option<Vec<u8>>> {
        let mut s = self.0.lock().unwrap();
        s.calls += 1;
        Ok(s.ac.get(&d.to_hex()).cloned())
    }
    async fn put(&self, d: &Digest, v: Vec<u8>) -> Result<()> {
        let mut s = self.0.lock().unwrap();
        s.calls += 1;
        assert!(!s.ac.contains_key(&d.to_hex()), "unexpected overwrite");
        s.ac.insert(d.to_hex(), v);
        Ok(())
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("run must preserve legacy records")
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        panic!("run does not enumerate refs")
    }
}

// A subprocess entrypoint, not an additional product scenario. The ordinary
// harness run only checks that it is inert without the explicit worker marker.
#[test]
fn environment_worker() {
    let Some(root) = std::env::var_os("CLW_302_WORKER_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let state_path = root.join("state.json");
    let mut state: State = if state_path.exists() {
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap()
    } else {
        State::default()
    };
    state.calls = 0;
    let store = Store(Mutex::new(state));
    let opts = RunOptions {
        inputs: vec![],
        command: command(),
        env_keys: vec![VAR.into()],
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = runtime.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run_memoized(root, &store, &opts),
        )
        .await
        .expect("bounded worker")
    });
    let result = match outcome {
        Ok(o) => {
            json!({"ok":true,"key":o.key.to_hex(),"hit":o.hit,"exit":o.exit_code,"stdout":o.stdout,"stderr":o.stderr})
        }
        Err(e) => {
            json!({"ok":false,"error":e.to_string(),"config_error":matches!(e,ClwError::Config(_))})
        }
    };
    let state = store.0.into_inner().unwrap();
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec(&json!({"outcome":result,"calls":state.calls})).unwrap(),
    )
    .unwrap();
    std::fs::write(state_path, serde_json::to_vec(&state).unwrap()).unwrap();
}

fn worker_command(root: &Path, value: Option<&[u8]>) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("CLW_302_WORKER_ROOT", root)
        .args(["--exact", "environment_worker", "--nocapture"]);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", profile);
    }
    if let Some(v) = value {
        c.env(VAR, OsString::from_vec(v.to_vec()));
    }
    c
}
fn invoke(root: &Path, value: Option<&[u8]>) -> Value {
    let o = worker_command(root, value).output().unwrap();
    assert!(
        o.status.success(),
        "worker failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&std::fs::read(root.join("result.json")).unwrap()).unwrap()
}
fn count(root: &Path) -> usize {
    std::fs::read(root.join("executions")).map_or(0, |v| v.len())
}

#[test]
fn supported_environment_values_keep_distinct_keys_and_real_hits() {
    let root = TempDir::new().unwrap();
    let values: &[Option<&[u8]>] = &[
        None,
        Some(b""),
        Some(b"A"),
        Some(b"B"),
        Some("café".as_bytes()),
    ];
    let mut keys = std::collections::BTreeSet::new();
    for (i, value) in values.iter().enumerate() {
        let first = invoke(root.path(), *value);
        assert_eq!(first["outcome"]["ok"], true, "{first}");
        assert_eq!(first["outcome"]["hit"], false, "{first}");
        assert_eq!(first["outcome"]["exit"], 0);
        assert!(keys.insert(first["outcome"]["key"].as_str().unwrap().to_string()));
        let expected = value.map_or_else(
            || b"unset".to_vec(),
            |v| [b"<".as_slice(), v, b">".as_slice()].concat(),
        );
        assert_eq!(first["outcome"]["stdout"], json!(expected));
        let repeated = invoke(root.path(), *value);
        assert_eq!(repeated["outcome"]["hit"], true, "{repeated}");
        assert_eq!(repeated["outcome"]["key"], first["outcome"]["key"]);
        assert_eq!(repeated["outcome"]["stdout"], first["outcome"]["stdout"]);
        assert_eq!(count(root.path()), i + 1);
    }
}

#[test]
fn invalid_declared_values_refuse_before_any_transport_or_child() {
    let root = TempDir::new().unwrap();
    let healthy = invoke(root.path(), None);
    assert_eq!(healthy["outcome"]["ok"], true);
    let before: State =
        serde_json::from_slice(&std::fs::read(root.path().join("state.json")).unwrap()).unwrap();
    for bytes in [
        b"synthetic-private-A\xff".as_slice(),
        b"synthetic-private-B\xfe".as_slice(),
    ] {
        let result = invoke(root.path(), Some(bytes));
        assert_eq!(
            result["outcome"]["ok"], false,
            "unsupported bytes authorized replay: {result}"
        );
        assert_eq!(result["outcome"]["config_error"], true);
        let error = result["outcome"]["error"].as_str().unwrap();
        assert!(error.contains(VAR) && error.contains("UTF-8"), "{error}");
        assert!(
            !error.contains("synthetic-private"),
            "value leaked in diagnostic"
        );
        assert_eq!(result["calls"], 0, "must reject before lookup");
        assert_eq!(count(root.path()), 1);
        let after: State =
            serde_json::from_slice(&std::fs::read(root.path().join("state.json")).unwrap())
                .unwrap();
        assert_eq!(after.ac, before.ac);
        assert_eq!(after.cas, before.cas);
    }
}

#[test]
fn legacy_v3_pollution_is_not_replayed_for_absent_environment() {
    // Frozen historical v3 wire payload. The original-code baseline must HIT
    // this exact seeded entry: that red run checks this fixture against the old
    // producer, instead of simply inventing an arbitrary unused key.
    #[derive(Serialize)]
    struct LegacyPayload {
        inputs: Vec<[String; 2]>,
        command: Vec<String>,
        env: Vec<[String; 2]>,
        tool: String,
        cwd: String,
    }
    let root = TempDir::new().unwrap();
    let payload = LegacyPayload {
        inputs: vec![],
        command: command(),
        env: vec![[VAR.into(), "unset".into()]],
        tool: format!(
            "digest:{}",
            Digest::of_bytes(&std::fs::read("/bin/sh").unwrap()).to_hex()
        ),
        cwd: std::fs::canonicalize(root.path())
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
    };
    let mut hash = blake3::Hasher::new();
    hash.update(b"clw/run/v3/");
    hash.update(&serde_json::to_vec(&payload).unwrap());
    let old_key = hash.finalize().to_hex().to_string();
    let old_stdout = b"<synthetic-private-A\xff>".to_vec();
    let out = Digest::of_bytes(&old_stdout);
    let err = Digest::of_bytes(b"");
    let record = serde_json::to_vec(&json!({"exit_code":0,"stdout":out,"stderr":err})).unwrap();
    let mut state = State::default();
    state.ac.insert(old_key.clone(), record.clone());
    state.cas.insert(out.to_hex(), old_stdout);
    state.cas.insert(err.to_hex(), vec![]);
    std::fs::write(
        root.path().join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let result = invoke(root.path(), None);
    assert_eq!(
        result["outcome"]["hit"], false,
        "legacy v3 pollution was replayed: {result}"
    );
    assert_ne!(result["outcome"]["key"], old_key);
    assert_eq!(result["outcome"]["stdout"], json!(b"unset".to_vec()));
    assert_eq!(count(root.path()), 1);
    let after: State =
        serde_json::from_slice(&std::fs::read(root.path().join("state.json")).unwrap()).unwrap();
    assert_eq!(after.ac.get(&old_key), Some(&record));
    assert!(clw_run::is_run_record(&record));
    assert_eq!(clw_run::run_record_blobs(&record), Some((out, err)));
    assert!(after.cas.contains_key(&out.to_hex()) && after.cas.contains_key(&err.to_hex()));
    assert_eq!(invoke(root.path(), None)["outcome"]["hit"], true);
}

#[test]
fn embedded_nul_is_refused_by_os_spawn_before_worker_executes() {
    let root = TempDir::new().unwrap();
    let error = worker_command(root.path(), Some(b"synthetic\0value"))
        .spawn()
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
