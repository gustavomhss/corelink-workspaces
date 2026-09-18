//! Real-CLI replay policy regression. Only loopback HTTP and synthetic data.
#![cfg(unix)]

use std::collections::HashMap;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tempfile::TempDir;
use wiremock::matchers::path_regex;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Default)]
struct State {
    cas: HashMap<String, Vec<u8>>,
    ac: HashMap<String, Vec<u8>>,
    cas_reads: usize,
    conflicts: usize,
    deletes: usize,
}

#[derive(Clone)]
struct Backend(Arc<Mutex<State>>);

impl Respond for Backend {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if request
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            != Some("Bearer test-pat")
        {
            return ResponseTemplate::new(401);
        }
        let parts: Vec<_> = request.url.path().split('/').collect();
        if parts.len() != 5 || parts[1] != "v1" || parts[3] != "test" {
            return ResponseTemplate::new(404);
        }
        let is_cas = parts[2] == "cas";
        let key = parts[4].to_owned();
        let mut state = self.0.lock().unwrap();
        match request.method.as_str() {
            "GET" | "HEAD" => {
                if is_cas {
                    state.cas_reads += 1;
                }
                let store = if is_cas { &state.cas } else { &state.ac };
                match store.get(&key) {
                    Some(bytes) if request.method.as_str() == "GET" => {
                        ResponseTemplate::new(200).set_body_bytes(bytes.clone())
                    }
                    Some(_) => ResponseTemplate::new(200),
                    None => ResponseTemplate::new(404),
                }
            }
            "PUT" => {
                if !is_cas && state.ac.contains_key(&key) {
                    state.conflicts += 1;
                    return ResponseTemplate::new(409);
                }
                let store = if is_cas {
                    &mut state.cas
                } else {
                    &mut state.ac
                };
                store.insert(key, request.body.clone());
                ResponseTemplate::new(201)
            }
            "DELETE" => {
                state.deletes += 1;
                ResponseTemplate::new(500)
            }
            _ => ResponseTemplate::new(405),
        }
    }
}

struct Fixture {
    server: MockServer,
    state: Arc<Mutex<State>>,
    root: TempDir,
    home: TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let state = Arc::new(Mutex::new(State::default()));
        Mock::given(path_regex(r"^/v1/(cas|ac)/test/[^/]+$"))
            .respond_with(Backend(state.clone()))
            .mount(&server)
            .await;
        let root = TempDir::new().unwrap();
        std::fs::write(root.path().join("input.txt"), b"stable input").unwrap();
        Self {
            server,
            state,
            root,
            home: TempDir::new().unwrap(),
        }
    }

    fn run(&self, code: i32) -> Output {
        Command::new(env!("CARGO_BIN_EXE_clw"))
            .current_dir(self.root.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home.path())
            .env("CLW_ENDPOINT", self.server.uri())
            .env("CLW_TENANT", "test")
            .env("CLW_TOKEN", "test-pat")
            .args(["run", "--input", "input.txt", "--", "/bin/sh", "-c"])
            .arg(format!(
                "printf x >> executions; printf out; printf err >&2; exit {code}"
            ))
            .output()
            .unwrap()
    }

    fn count(&self) -> usize {
        std::fs::read(self.root.path().join("executions")).map_or(0, |b| b.len())
    }

    fn record(&self) -> (String, Vec<u8>) {
        let state = self.state.lock().unwrap();
        assert_eq!(state.ac.len(), 1, "the real producer must publish one memo");
        state
            .ac
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), v.clone()))
            .unwrap()
    }

    fn replace_exit(&self, code: i32) -> (String, Vec<u8>, Vec<u8>) {
        let (key, original) = self.record();
        let mut value: Value = serde_json::from_slice(&original).unwrap();
        value["exit_code"] = Value::from(code);
        let changed = serde_json::to_vec(&value).unwrap();
        self.state
            .lock()
            .unwrap()
            .ac
            .insert(key.clone(), changed.clone());
        (key, original, changed)
    }
}

fn assert_outcome(output: &Output, code: i32, hit: bool) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert_eq!(output.stdout, b"out", "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.ends_with("err"), "{output:?}");
    assert!(
        stderr.contains(if hit {
            "[clw] cache hit"
        } else {
            "[clw] cache miss"
        }),
        "{output:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonzero_record_is_a_miss_without_replacing_its_gc_references() {
    let f = Fixture::new().await;
    assert_outcome(&f.run(0), 0, false);
    assert_eq!(f.count(), 1);
    let (key, original, changed) = f.replace_exit(42);
    let before = clw_run::run_record_blobs(&original).expect("produced record has streams");
    let after = clw_run::run_record_blobs(&changed).expect("ineligible record still roots streams");
    assert_eq!(before, after);
    let reads = f.state.lock().unwrap().cas_reads;
    for expected_count in [2, 3] {
        assert_outcome(&f.run(0), 0, false);
        assert_eq!(
            f.count(),
            expected_count,
            "one fallback child per invocation"
        );
        let state = f.state.lock().unwrap();
        assert_eq!(
            state.ac.get(&key),
            Some(&changed),
            "never replace the AC occupant"
        );
        assert_eq!(
            state.conflicts,
            expected_count - 1,
            "create-only store must reject"
        );
        assert_eq!(
            state.deletes, 0,
            "read eligibility must not trigger repair deletes"
        );
        assert_eq!(
            state.cas_reads, reads,
            "ineligible replay needs no stream reads"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_record_replays_exact_streams_without_another_child() {
    let f = Fixture::new().await;
    assert_outcome(&f.run(0), 0, false);
    assert_outcome(&f.run(0), 0, true);
    assert_eq!(f.count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_child_is_not_memoized() {
    let f = Fixture::new().await;
    for expected_count in [1, 2] {
        assert_outcome(&f.run(42), 42, false);
        assert_eq!(f.count(), expected_count);
        assert!(f.state.lock().unwrap().ac.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_record_keeps_the_existing_single_fallback_policy() {
    let f = Fixture::new().await;
    assert_outcome(&f.run(0), 0, false);
    let (key, _) = f.record();
    f.state
        .lock()
        .unwrap()
        .ac
        .insert(key.clone(), b"{}".to_vec());
    assert_outcome(&f.run(0), 0, false);
    assert_eq!(f.count(), 2);
    let state = f.state.lock().unwrap();
    assert_eq!(state.ac[&key], b"{}");
    assert_eq!(state.conflicts, 1);
    assert_eq!(state.deletes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_stream_falls_back_then_healthy_record_hits_again() {
    let f = Fixture::new().await;
    assert_outcome(&f.run(0), 0, false);
    let (key, original) = f.record();
    f.state.lock().unwrap().cas.clear();
    assert_outcome(&f.run(0), 0, false);
    assert_eq!(f.count(), 2);
    assert_eq!(f.state.lock().unwrap().ac[&key], original);
    assert_outcome(&f.run(0), 0, true);
    assert_eq!(f.count(), 2);
    assert_eq!(f.state.lock().unwrap().deletes, 0);
}
