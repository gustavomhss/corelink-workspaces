//! Reviewer regressions for the #301 resolve/lookup/dispatch/store boundary.
//! Separate worker processes provide actual PATH; only CAS/AC are controlled.
#![cfg(unix)]
use clw_run::{run_memoized, RunOptions};
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use tempfile::TempDir;
#[derive(Default, Serialize, Deserialize)]
struct State {
    cas: BTreeMap<String, Vec<u8>>,
    ac: BTreeMap<String, Vec<u8>>,
    reads: usize,
    writes: usize,
}
struct Store {
    state: Mutex<State>,
    root: PathBuf,
    hook: Mutex<Option<String>>,
    // Per worker invocation, not persisted as product state.
    stream_reads: Mutex<usize>,
}
impl Store {
    fn take_hook(&self, expected: &str) -> bool {
        let mut hook = self.hook.lock().unwrap();
        if hook.as_deref() == Some(expected) {
            *hook = None;
            true
        } else {
            false
        }
    }
}
#[async_trait::async_trait]
impl CasTransport for Store {
    async fn exists(&self, d: &Digest) -> Result<bool> {
        Ok(self.state.lock().unwrap().cas.contains_key(&d.to_hex()))
    }
    async fn get(&self, d: &Digest) -> Result<Vec<u8>> {
        let bytes = self
            .state
            .lock()
            .unwrap()
            .cas
            .get(&d.to_hex())
            .cloned()
            .ok_or(ClwError::NotFound(*d))?;
        // The run record has stdout and stderr. Change the selected tool while
        // returning the second stream, after any check made just after AC GET.
        let is_last_stream = {
            let mut reads = self.stream_reads.lock().unwrap();
            *reads += 1;
            *reads == 2
        };
        if is_last_stream && self.take_hook("replace-selected-on-last-stream") {
            write_tool(&self.root.join("bin/probe"), "new", false);
        }
        Ok(bytes)
    }
    async fn put(&self, d: &Digest, v: Vec<u8>) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        s.writes += 1;
        s.cas.insert(d.to_hex(), v);
        Ok(())
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("run cannot delete")
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        panic!("run cannot list")
    }
}
#[async_trait::async_trait]
impl AcTransport for Store {
    async fn get(&self, d: &Digest) -> Result<Option<Vec<u8>>> {
        // A CAS-stage hook must survive the earlier Action Cache lookup.
        if self.take_hook("promote-shadow") {
            std::fs::set_permissions(
                self.root.join("shadow/probe"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        if self.take_hook("replace-selected") {
            write_tool(&self.root.join("bin/probe"), "new", false);
        }
        let mut s = self.state.lock().unwrap();
        s.reads += 1;
        Ok(s.ac.get(&d.to_hex()).cloned())
    }
    async fn put(&self, d: &Digest, v: Vec<u8>) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        s.writes += 1;
        assert!(s.ac.insert(d.to_hex(), v).is_none());
        Ok(())
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("run cannot delete")
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        panic!("run cannot list")
    }
}
fn write_tool(p: &Path, word: &str, edit: bool) {
    let tail = if edit {
        "printf '# changed after dispatch\\n' >> \"$0\"\n"
    } else {
        ""
    };
    std::fs::write(
        p,
        format!("#!/bin/sh\nprintf '{word}'\nprintf x >> effects\n{tail}"),
    )
    .unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}
#[test]
fn binding_worker() {
    let Some(root) = std::env::var_os("CLW301_BIND_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let state = std::fs::read(root.join("store.json"))
        .ok()
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap_or_default();
    let store = Store {
        state: Mutex::new(state),
        root: root.clone(),
        hook: Mutex::new(std::env::var("CLW301_BIND_HOOK").ok()),
        stream_reads: Mutex::new(0),
    };
    let opts = RunOptions {
        inputs: vec!["input.txt".into()],
        command: vec!["probe".into()],
        env_keys: vec!["PATH".into()],
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run_memoized(&root, &store, &opts),
        )
        .await
        .expect("bounded invocation")
    });
    let result = match result {
        Ok(v) => {
            json!({"ok":true,"hit":v.hit,"key":v.key.to_hex(),"stdout":v.stdout,"exit":v.exit_code})
        }
        Err(e) => json!({"ok":false,"error":e.to_string()}),
    };
    let state = store.state.into_inner().unwrap();
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("store.json"), serde_json::to_vec(&state).unwrap()).unwrap();
}
fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    for p in ["shadow", "bin"] {
        std::fs::create_dir(dir.path().join(p)).unwrap();
    }
    std::fs::write(dir.path().join("input.txt"), b"constant").unwrap();
    dir
}
fn invoke(root: &Path, hook: Option<&str>) -> Value {
    let result = root.join("result.json");
    if result.exists() {
        std::fs::remove_file(&result).unwrap();
    }
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.env_clear()
        .env("HOME", root)
        .env(
            "PATH",
            std::env::join_paths([root.join("shadow"), root.join("bin")]).unwrap(),
        )
        .env("CLW301_BIND_ROOT", root)
        .args(["--exact", "binding_worker", "--nocapture"]);
    if let Some(v) = hook {
        cmd.env("CLW301_BIND_HOOK", v);
    }
    if let Some(v) = std::env::var_os("LLVM_PROFILE_FILE") {
        cmd.env("LLVM_PROFILE_FILE", v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&std::fs::read(root.join("result.json")).unwrap()).unwrap()
}
fn state(root: &Path) -> State {
    serde_json::from_slice(&std::fs::read(root.join("store.json")).unwrap()).unwrap()
}
#[test]
fn path_promotion_during_lookup_does_not_reselect_the_launch() {
    let dir = fixture();
    let root = dir.path();
    let shadow = root.join("shadow/probe");
    write_tool(&shadow, "first", false);
    std::fs::set_permissions(&shadow, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_tool(&root.join("bin/probe"), "second", false);
    let first = invoke(root, Some("promote-shadow"));
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["stdout"], json!(b"second"));
    assert_eq!(first["hit"], false);
    let next = invoke(root, None);
    assert_eq!(next["stdout"], json!(b"first"));
    assert_eq!(next["hit"], false);
    assert_ne!(next["key"], first["key"]);
    assert_eq!(invoke(root, None)["hit"], true);
    assert_eq!(std::fs::read(root.join("effects")).unwrap(), b"xx");
}
#[test]
fn changed_selected_tool_at_lookup_is_refused_without_dispatch_or_store() {
    let dir = fixture();
    let root = dir.path();
    write_tool(&root.join("bin/probe"), "old", false);
    let result = invoke(root, Some("replace-selected"));
    assert_eq!(result["ok"], false, "{result}");
    assert!(!root.join("effects").exists());
    let s = state(root);
    assert_eq!(s.reads, 1);
    assert_eq!(s.writes, 0);
    assert!(s.ac.is_empty() && s.cas.is_empty());
    let healthy = invoke(root, None);
    assert_eq!(healthy["ok"], true);
    assert_eq!(healthy["stdout"], json!(b"new"));
    assert_eq!(invoke(root, None)["hit"], true);
}
#[test]
fn detected_post_dispatch_tool_change_preserves_result_but_not_a_memo() {
    let dir = fixture();
    let root = dir.path();
    write_tool(&root.join("bin/probe"), "observed", true);
    for _ in 0..2 {
        let result = invoke(root, None);
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["stdout"], json!(b"observed"));
        assert_eq!(result["exit"], 0);
        assert_eq!(result["hit"], false);
        let s = state(root);
        assert_eq!(s.writes, 0);
        assert!(s.ac.is_empty() && s.cas.is_empty());
    }
    assert_eq!(std::fs::read(root.join("effects")).unwrap(), b"xx");
}

// A warm cache must not skip the selected-tool stability check. The cold
// dispatch regression above cannot exercise the early successful-HIT return.
fn assert_selected_tool_change_rejects_warm_replay(hook: &str) {
    let dir = fixture();
    let root = dir.path();
    let tool = root.join("bin/probe");
    write_tool(&tool, "old", false);
    let tool_before = std::fs::read(&tool).unwrap();
    let first = invoke(root, None);
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["hit"], false);
    assert_eq!(first["stdout"], json!(b"old"));
    assert_eq!(first["exit"], 0);
    let repeated = invoke(root, None);
    assert_eq!(repeated["ok"], true, "{repeated}");
    assert_eq!(repeated["hit"], true);
    assert_eq!(repeated["key"], first["key"]);
    assert_eq!(repeated["stdout"], first["stdout"]);
    assert_eq!(std::fs::read(root.join("effects")).unwrap(), b"x");
    let before = state(root);

    let result = invoke(root, Some(hook));
    // Prove the deterministic timing hook actually fired. A skipped CAS hook
    // must not turn this test into a purported stable-tool control.
    assert_ne!(
        std::fs::read(&tool).unwrap(),
        tool_before,
        "hook not reached"
    );
    assert_eq!(
        result["ok"], false,
        "changed tool authorized replay: {result}"
    );
    assert!(
        result.get("stdout").is_none(),
        "refusal exposed cached output"
    );
    assert_eq!(std::fs::read(root.join("effects")).unwrap(), b"x");
    let after = state(root);
    assert_eq!(after.reads, before.reads + 1);
    assert_eq!(after.writes, before.writes);
    assert_eq!(after.ac, before.ac, "refusal must not replace legacy memos");
    assert_eq!(after.cas, before.cas, "refusal must preserve stream blobs");

    // Explicit next invocation, not an automatic retry inside the failed run.
    // It resolves the now-stable new executable, executes once, then can HIT.
    let healthy = invoke(root, None);
    assert_eq!(healthy["ok"], true, "{healthy}");
    assert_eq!(healthy["hit"], false);
    assert_eq!(healthy["stdout"], json!(b"new"));
    assert_eq!(healthy["exit"], 0);
    assert_ne!(healthy["key"], first["key"]);
    let again = invoke(root, None);
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(again["hit"], true);
    assert_eq!(again["key"], healthy["key"]);
    assert_eq!(again["stdout"], healthy["stdout"]);
    assert_eq!(std::fs::read(root.join("effects")).unwrap(), b"xx");
}

#[test]
fn changed_selected_tool_on_warm_ac_hit_is_refused_before_replay() {
    assert_selected_tool_change_rejects_warm_replay("replace-selected");
}

#[test]
fn changed_selected_tool_during_last_stream_is_refused_before_replay() {
    assert_selected_tool_change_rejects_warm_replay("replace-selected-on-last-stream");
}
