//! #301: real tool permissions, native command selection, keys and child effects.
//! Workers isolate process environment; no test mutates the runner's PATH.
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
        panic!("run must not delete blobs")
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        panic!("run must not list blobs")
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
        s.ac.entry(d.to_hex()).or_insert(v);
        Ok(())
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("run must preserve old records")
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        panic!("run must not list refs")
    }
}

// Inert in a normal harness run; the parent explicitly starts this worker.
#[test]
fn tool_worker() {
    let Some(root) = std::env::var_os("CLW301_WORKER") else {
        return;
    };
    let root = Path::new(&root);
    let mut state: State = std::fs::read(root.join("store.json"))
        .ok()
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap_or_default();
    state.calls = 0;
    let store = Store(Mutex::new(state));
    let command: Vec<String> =
        serde_json::from_slice(&std::fs::read(root.join("command.json")).unwrap()).unwrap();
    let opts = RunOptions {
        inputs: vec!["input.txt".into()],
        command,
        env_keys: vec!["PATH".into()],
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_memoized(root, &store, &opts),
        )
        .await
        .expect("bounded worker")
    });
    let outcome = match result {
        Ok(o) => {
            json!({"ok":true,"key":o.key.to_hex(),"hit":o.hit,"exit":o.exit_code,"stdout":o.stdout,"stderr":o.stderr})
        }
        Err(e) => {
            let io_kind = match &e {
                ClwError::Io(error) => Some(format!("{:?}", error.kind())),
                _ => None,
            };
            json!({"ok":false,"error":e.to_string(),"io_kind":io_kind})
        }
    };
    let state = store.0.into_inner().unwrap();
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec(&json!({"outcome":outcome,"calls":state.calls})).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("store.json"), serde_json::to_vec(&state).unwrap()).unwrap();
}

// Native binary probe: unlike a shell script, it observes the passed argv[0].
#[test]
fn argv_probe() {
    let Some(root) = std::env::var_os("CLW301_ARGV_PROBE") else {
        return;
    };
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(&root).join("effects"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    let args: Vec<String> = std::env::args().collect();
    std::fs::write(
        Path::new(&root).join("argv.json"),
        serde_json::to_vec(&json!({"args":args,"cwd":std::env::current_dir().unwrap()})).unwrap(),
    )
    .unwrap();
}

struct Fixture {
    temp: TempDir,
    path: std::ffi::OsString,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let r = temp.path();
        std::fs::create_dir(r.join("shadow")).unwrap();
        std::fs::create_dir(r.join("bin")).unwrap();
        std::fs::write(r.join("input.txt"), b"stable input").unwrap();
        let path = std::env::join_paths([
            r.join("shadow"),
            r.join("bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ])
        .unwrap();
        let f = Self { temp, path };
        f.command(&["clw301-probe"]);
        f
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    fn command(&self, args: &[&str]) {
        std::fs::write(
            self.root().join("command.json"),
            serde_json::to_vec(args).unwrap(),
        )
        .unwrap();
    }
    fn script(&self, dir: &str, output: &str, mode: u32) -> PathBuf {
        let p = self.root().join(dir).join("clw301-probe");
        std::fs::write(
            &p,
            format!("#!/bin/sh\nprintf '{output}'\nprintf x >> effects\n"),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }
    fn invoke(&self) -> Value {
        self.invoke_with_path(Some(&self.path))
    }
    fn invoke_with_path(&self, path: Option<&std::ffi::OsStr>) -> Value {
        let result = self.root().join("result.json");
        if result.exists() {
            std::fs::remove_file(&result).unwrap();
        }
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.env_clear()
            .env("HOME", self.root())
            .env("CLW301_WORKER", self.root())
            .env("CLW301_ARGV_PROBE", self.root())
            .args(["--exact", "tool_worker", "--nocapture"]);
        if let Some(path) = path {
            c.env("PATH", path);
        }
        if let Some(v) = std::env::var_os("LLVM_PROFILE_FILE") {
            c.env("LLVM_PROFILE_FILE", v);
        }
        let o = c.output().unwrap();
        assert!(
            o.status.success(),
            "worker: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        serde_json::from_slice(&std::fs::read(self.root().join("result.json")).unwrap()).unwrap()
    }
    fn effects(&self) -> usize {
        std::fs::read(self.root().join("effects")).map_or(0, |b| b.len())
    }
    fn direct(&self) -> Vec<u8> {
        let o = Command::new("clw301-probe")
            .current_dir(self.root())
            .env_clear()
            .env("PATH", &self.path)
            .output()
            .unwrap();
        assert!(o.status.success());
        o.stdout
    }
}
fn success(r: &Value, bytes: &[u8], hit: bool) {
    assert_eq!(r["outcome"]["ok"], true, "{r}");
    assert_eq!(r["outcome"]["exit"], 0, "{r}");
    assert_eq!(r["outcome"]["stdout"], json!(bytes), "{r}");
    assert_eq!(r["outcome"]["hit"], hit, "{r}");
}

#[test]
fn non_executable_path_shadow_cannot_hide_actual_tool_change() {
    let f = Fixture::new();
    f.script("shadow", "shadow", 0o644);
    f.script("bin", "old", 0o755);
    let first = f.invoke();
    success(&first, b"old", false);
    success(&f.invoke(), b"old", true);
    assert_eq!(f.effects(), 1);
    f.script("bin", "new", 0o755);
    assert_eq!(f.direct(), b"new");
    let second = f.invoke();
    success(&second, b"new", false);
    assert_ne!(first["outcome"]["key"], second["outcome"]["key"]);
    assert_eq!(f.effects(), 3);
    success(&f.invoke(), b"new", true);
    assert_eq!(f.effects(), 3);
}

#[test]
fn permission_change_switches_selected_tool_and_retains_healthy_hits() {
    let f = Fixture::new();
    let shadow = f.script("shadow", "first", 0o644);
    f.script("bin", "second", 0o755);
    let a = f.invoke();
    success(&a, b"second", false);
    std::fs::set_permissions(&shadow, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(f.direct(), b"first");
    let b = f.invoke();
    success(&b, b"first", false);
    assert_ne!(a["outcome"]["key"], b["outcome"]["key"]);
    success(&f.invoke(), b"first", true);
}

#[test]
fn symlinked_tool_and_no_shadow_controls_follow_actual_output() {
    let f = Fixture::new();
    let a = f.script("shadow", "alpha", 0o755);
    let b = f.script("bin", "bravo", 0o755);
    let link = f.root().join("linked-tool");
    std::os::unix::fs::symlink(&a, &link).unwrap();
    f.command(&[link.to_str().unwrap()]);
    let first = f.invoke();
    success(&first, b"alpha", false);
    success(&f.invoke(), b"alpha", true);
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&b, &link).unwrap();
    let second = f.invoke();
    success(&second, b"bravo", false);
    assert_ne!(first["outcome"]["key"], second["outcome"]["key"]);
    std::fs::remove_file(&a).unwrap();
    f.command(&["clw301-probe"]);
    success(&f.invoke(), b"bravo", false);
    success(&f.invoke(), b"bravo", true);
}

#[test]
fn missing_tool_never_authorizes_weak_identity_lookup() {
    let f = Fixture::new();
    let missing = f.invoke();
    assert_eq!(missing["outcome"]["ok"], false);
    assert_eq!(missing["outcome"]["io_kind"], "NotFound", "{missing}");
    assert_eq!(
        missing["calls"], 0,
        "missing tool must not consult a memo: {missing}"
    );
    assert_eq!(f.effects(), 0);
}

#[test]
fn execute_only_tool_never_authorizes_weak_identity_lookup() {
    let f = Fixture::new();
    let executable = f.root().join("bin/clw301-probe");
    std::fs::copy("/bin/echo", &executable).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o111)).unwrap();
    assert!(
        std::fs::read(&executable).is_err(),
        "fixture needs effective unreadability"
    );
    // macOS may refuse an execute-only Mach-O at the loader, despite execute
    // permission. The contract here is pre-lookup refusal of unreadable identity,
    // not a claim that every platform can launch this specimen.
    let rejected = f.invoke();
    assert_eq!(
        rejected["outcome"]["ok"], false,
        "unidentifiable tool must fail before memoization: {rejected}"
    );
    assert_eq!(rejected["calls"], 0);
}

#[test]
fn native_argv0_arguments_and_cwd_are_preserved() {
    let f = Fixture::new();
    let alias = f.root().join("bin/clw301-probe");
    std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &alias).unwrap();
    f.command(&["clw301-probe", "--exact", "argv_probe", "--nocapture"]);
    let first = f.invoke();
    assert_eq!(first["outcome"]["ok"], true, "{first}");
    assert_eq!(first["outcome"]["hit"], false);
    let observed: Value =
        serde_json::from_slice(&std::fs::read(f.root().join("argv.json")).unwrap()).unwrap();
    assert_eq!(
        observed["args"],
        json!(["clw301-probe", "--exact", "argv_probe", "--nocapture"])
    );
    assert_eq!(
        Path::new(observed["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        f.root().canonicalize().unwrap()
    );
    assert_eq!(f.invoke()["outcome"]["hit"], true);
    assert_eq!(f.effects(), 1, "native argv probe must execute only once");
}

#[test]
fn frozen_v4_wrong_tool_record_is_not_reused_after_selection_fix() {
    #[derive(Serialize)]
    struct LegacyPayload {
        inputs: Vec<[String; 2]>,
        command: Vec<String>,
        env: Vec<[String; 2]>,
        tool: String,
        cwd: String,
    }
    let f = Fixture::new();
    let executable = f.script("shadow", "correct", 0o755);
    f.script("bin", "obsolete", 0o755);
    let input = f.root().join("input.txt");
    let mode = std::fs::metadata(&input).unwrap().permissions().mode() & 0o777;
    let payload = LegacyPayload {
        inputs: vec![[
            "input.txt".into(),
            format!(
                "{}:{mode:o}",
                Digest::of_bytes(&std::fs::read(&input).unwrap()).to_hex()
            ),
        ]],
        command: vec!["clw301-probe".into()],
        env: vec![["PATH".into(), format!("set:{}", f.path.to_str().unwrap())]],
        tool: format!(
            "digest:{}",
            Digest::of_bytes(&std::fs::read(&executable).unwrap()).to_hex()
        ),
        cwd: f.root().canonicalize().unwrap().to_str().unwrap().into(),
    };
    let mut h = blake3::Hasher::new();
    h.update(b"clw/run/v4/");
    h.update(&serde_json::to_vec(&payload).unwrap());
    let old_key = h.finalize().to_hex().to_string();
    let out = Digest::of_bytes(b"obsolete");
    let err = Digest::of_bytes(b"");
    let record = serde_json::to_vec(&json!({"exit_code":0,"stdout":out,"stderr":err})).unwrap();
    let mut state = State::default();
    state.ac.insert(old_key.clone(), record.clone());
    state.cas.insert(out.to_hex(), b"obsolete".to_vec());
    state.cas.insert(err.to_hex(), vec![]);
    std::fs::write(
        f.root().join("store.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let result = f.invoke();
    success(&result, b"correct", false);
    assert_ne!(result["outcome"]["key"], old_key);
    assert_eq!(f.effects(), 1);
    let after: State =
        serde_json::from_slice(&std::fs::read(f.root().join("store.json")).unwrap()).unwrap();
    assert_eq!(after.ac.get(&old_key), Some(&record));
    assert_eq!(clw_run::run_record_blobs(&record), Some((out, err)));
    assert!(after.cas.contains_key(&out.to_hex()));
}

#[test]
fn absent_path_requires_explicit_tool_without_guessing_native_defaults() {
    let f = Fixture::new();
    let tool = f.script("bin", "explicit", 0o755);
    f.command(&[tool.to_str().unwrap()]);
    success(&f.invoke_with_path(None), b"explicit", false);
    success(&f.invoke_with_path(None), b"explicit", true);
    assert_eq!(f.effects(), 1);
    f.command(&["clw301-probe"]);
    let result = f.invoke_with_path(None);
    assert_eq!(result["outcome"]["ok"], false);
    assert_eq!(result["calls"], 0);
    assert_eq!(f.effects(), 1);
}

#[test]
fn relative_program_and_relative_path_use_the_child_cwd() {
    let f = Fixture::new();
    f.script("bin", "relative", 0o755);
    f.command(&["./bin/clw301-probe"]);
    success(&f.invoke_with_path(None), b"relative", false);
    success(&f.invoke_with_path(None), b"relative", true);
    f.command(&["clw301-probe"]);
    let relative_path = std::ffi::OsStr::new("bin");
    success(&f.invoke_with_path(Some(relative_path)), b"relative", false);
    success(&f.invoke_with_path(Some(relative_path)), b"relative", true);
    assert_eq!(f.effects(), 2);
}

#[test]
fn empty_path_binds_to_the_child_cwd() {
    let f = Fixture::new();
    f.script("", "local", 0o755);
    let empty = std::ffi::OsStr::new("");
    success(&f.invoke_with_path(Some(empty)), b"local", false);
    success(&f.invoke_with_path(Some(empty)), b"local", true);
    assert_eq!(f.effects(), 1);
}

#[test]
fn removing_execution_permission_selects_the_next_valid_tool() {
    let f = Fixture::new();
    let first = f.script("shadow", "first", 0o755);
    f.script("bin", "second", 0o755);
    let a = f.invoke();
    success(&a, b"first", false);
    std::fs::set_permissions(first, std::fs::Permissions::from_mode(0o644)).unwrap();
    let b = f.invoke();
    success(&b, b"second", false);
    assert_ne!(a["outcome"]["key"], b["outcome"]["key"]);
    success(&f.invoke(), b"second", true);
    assert_eq!(f.effects(), 2);
}

#[test]
fn directory_and_dangling_symlink_candidates_do_not_hide_the_executable() {
    let f = Fixture::new();
    let first = f.root().join("shadow/clw301-probe");
    std::fs::create_dir(&first).unwrap();
    f.script("bin", "selected", 0o755);
    success(&f.invoke(), b"selected", false);
    std::fs::remove_dir(&first).unwrap();
    std::os::unix::fs::symlink("missing-target", &first).unwrap();
    success(&f.invoke(), b"selected", true);
    assert_eq!(f.effects(), 1);
}

#[test]
fn script_launch_spelling_matches_native_direct_execution() {
    for variant in ["relative-path", "empty-path", "explicit-relative"] {
        let f = Fixture::new();
        let target = f.root().join("script-target");
        std::fs::write(
            &target,
            b"#!/bin/sh\nprintf '%s' \"$0\"\nprintf x >> effects\n",
        )
        .unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (alias, path, program) = if variant == "empty-path" {
            (f.root().join("clw301-probe"), "", "clw301-probe")
        } else if variant == "explicit-relative" {
            (
                f.root().join("bin/clw301-probe"),
                "bin",
                "./bin/clw301-probe",
            )
        } else {
            (f.root().join("bin/clw301-probe"), "bin", "clw301-probe")
        };
        std::os::unix::fs::symlink(&target, alias).unwrap();
        f.command(&[program]);
        let direct = Command::new(program)
            .current_dir(f.root())
            .env_clear()
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(direct.status.success(), "native launch failed: {direct:?}");
        assert!(
            !direct.stdout.is_empty(),
            "script must report its launch spelling"
        );
        let first = f.invoke_with_path(Some(std::ffi::OsStr::new(path)));
        success(&first, &direct.stdout, false);
        let repeated = f.invoke_with_path(Some(std::ffi::OsStr::new(path)));
        success(&repeated, &direct.stdout, true);
        assert_eq!(first["outcome"]["key"], repeated["outcome"]["key"]);
        assert_eq!(
            f.effects(),
            2,
            "one native oracle and one memoized child; no extra discovery"
        );
    }
}

#[test]
fn cwd_only_path_preserves_script_spelling_and_child_environment() {
    for path in ["", ":", "::"] {
        let f = Fixture::new();
        let script = f.root().join("clw301-probe");
        std::fs::write(
            &script,
            b"#!/bin/sh\nprintf '%s|%s' \"$0\" \"$PATH\"\nprintf x >> effects\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let direct = Command::new("clw301-probe")
            .current_dir(f.root())
            .env_clear()
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(direct.status.success(), "native oracle failed: {direct:?}");
        assert!(direct.stdout.ends_with(format!("|{path}").as_bytes()));
        let first = f.invoke_with_path(Some(std::ffi::OsStr::new(path)));
        success(&first, &direct.stdout, false);
        let repeated = f.invoke_with_path(Some(std::ffi::OsStr::new(path)));
        success(&repeated, &direct.stdout, true);
        assert_eq!(first["outcome"]["key"], repeated["outcome"]["key"]);
        assert_eq!(f.effects(), 2, "one native oracle plus one memoized child");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn selected_mixed_empty_path_refuses_before_lookup_with_explicit_path_recovery() {
    for path in [":bin", "missing:", "missing::bin"] {
        let f = Fixture::new();
        f.script("", "local", 0o755);
        f.script("bin", "later", 0o755);
        let direct = Command::new("clw301-probe")
            .current_dir(f.root())
            .env_clear()
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(direct.status.success());
        assert_eq!(direct.stdout, b"local");
        let refused = f.invoke_with_path(Some(std::ffi::OsStr::new(path)));
        assert_eq!(refused["outcome"]["ok"], false, "{refused}");
        assert!(refused["outcome"]["error"]
            .as_str()
            .unwrap()
            .contains("explicit ./tool"));
        assert_eq!(refused["calls"], 0);
        assert_eq!(f.effects(), 1, "the refusal must not dispatch a child");
        f.command(&["./clw301-probe"]);
        success(
            &f.invoke_with_path(Some(std::ffi::OsStr::new(path))),
            b"local",
            false,
        );
        success(
            &f.invoke_with_path(Some(std::ffi::OsStr::new(path))),
            b"local",
            true,
        );
        assert_eq!(f.effects(), 2);
    }
    // A nonempty winning entry remains supported, even with later empty entries.
    let f = Fixture::new();
    f.script("", "local", 0o755);
    f.script("bin", "first", 0o755);
    let path = std::ffi::OsStr::new("bin:");
    success(&f.invoke_with_path(Some(path)), b"first", false);
    success(&f.invoke_with_path(Some(path)), b"first", true);
    assert_eq!(f.effects(), 1);
}

// A non-searchable path is neither a missing entry nor an ordinary permission
// refusal. Do not silently continue to a later executable after that error.
#[test]
fn unexpected_path_lookup_error_does_not_fall_through_to_another_tool() {
    let f = Fixture::new();
    let loop_path = f.root().join("shadow/clw301-probe");
    std::os::unix::fs::symlink("clw301-probe", &loop_path).unwrap();
    let expected = std::fs::metadata(&loop_path).unwrap_err();
    assert_ne!(expected.kind(), std::io::ErrorKind::NotFound);
    assert_ne!(expected.kind(), std::io::ErrorKind::NotADirectory);
    assert_ne!(expected.kind(), std::io::ErrorKind::PermissionDenied);
    f.script("bin", "must-not-run", 0o755);

    let result = f.invoke();
    assert_eq!(result["outcome"]["ok"], false, "{result}");
    assert_eq!(
        result["outcome"]["io_kind"],
        format!("{:?}", expected.kind()),
        "the actual filesystem error must remain distinct: {result}"
    );
    assert_eq!(result["calls"], 0, "no weak selection may reach a memo");
    assert_eq!(f.effects(), 0, "the later tool must not be dispatched");
}

#[test]
fn all_denied_path_candidates_report_permission_failure_without_lookup() {
    let f = Fixture::new();
    f.script("shadow", "denied-first", 0o644);
    f.script("bin", "denied-second", 0o644);
    let result = f.invoke();
    assert_eq!(result["outcome"]["ok"], false, "{result}");
    assert_eq!(result["outcome"]["io_kind"], "PermissionDenied", "{result}");
    assert_eq!(result["calls"], 0);
    assert_eq!(f.effects(), 0);
}
