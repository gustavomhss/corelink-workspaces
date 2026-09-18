//! Preserve memo stream roots even when a stored result is ineligible for replay.
//!
//! The real CLI produces the workspace and memo, lists the stores and performs
//! prune. Only the HTTP stores are doubles; all bytes and credentials are local
//! synthetic fixtures. No request is sent to the default application endpoint.
#![cfg(unix)]

use std::collections::HashMap;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use clw_client::HttpClient;
use clw_types::{CasTransport, Config, Digest};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::path_regex;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Default)]
struct State {
    cas: HashMap<String, Vec<u8>>,
    ac: HashMap<String, Vec<u8>>,
    requests: Vec<(String, String)>,
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
        if !(parts.len() == 4 || parts.len() == 5)
            || parts[1] != "v1"
            || !matches!(parts[2], "cas" | "ac")
            || parts[3] != "test"
        {
            return ResponseTemplate::new(404);
        }
        let is_cas = parts[2] == "cas";
        let method = request.method.as_str();
        let mut state = self.0.lock().unwrap();
        state
            .requests
            .push((method.to_owned(), request.url.path().to_owned()));

        if parts.len() == 4 {
            if method != "GET" {
                return ResponseTemplate::new(405);
            }
            let store = if is_cas { &state.cas } else { &state.ac };
            let mut keys: Vec<_> = store.keys().cloned().collect();
            keys.sort();
            let rows: Vec<Value> = keys
                .iter()
                .map(|key| {
                    if is_cas {
                        json!({ "hash": key })
                    } else {
                        json!({ "ref_key": key })
                    }
                })
                .collect();
            let body = if is_cas {
                json!({ "blobs": rows, "next_cursor": null })
            } else {
                json!({ "refs": rows, "next_cursor": null })
            };
            return ResponseTemplate::new(200).set_body_json(body);
        }

        let key = parts[4];
        match method {
            "GET" | "HEAD" => {
                let store = if is_cas { &state.cas } else { &state.ac };
                match store.get(key) {
                    Some(bytes) if method == "GET" => {
                        ResponseTemplate::new(200).set_body_bytes(bytes.clone())
                    }
                    Some(_) => ResponseTemplate::new(200),
                    None => ResponseTemplate::new(404),
                }
            }
            "PUT" => {
                // Preserve the real create-only AC contract. A permissive upsert
                // would hide unintended changes to the stored memo.
                if !is_cas && state.ac.contains_key(key) {
                    return ResponseTemplate::new(409);
                }
                if is_cas && Digest::of_bytes(&request.body).to_hex() != key {
                    return ResponseTemplate::new(400);
                }
                let store = if is_cas {
                    &mut state.cas
                } else {
                    &mut state.ac
                };
                store.insert(key.to_owned(), request.body.clone());
                ResponseTemplate::new(201)
            }
            "DELETE" if is_cas => {
                // This removes only bytes in this fixture's in-memory map.
                state.cas.remove(key);
                ResponseTemplate::new(204)
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
        Mock::given(path_regex(r"^/v1/(cas|ac)/test(?:/[^/]+)?$"))
            .respond_with(Backend(state.clone()))
            .mount(&server)
            .await;
        let root = TempDir::new().unwrap();
        std::fs::write(root.path().join("input.txt"), b"workspace control\n").unwrap();
        Self {
            server,
            state,
            root,
            home: TempDir::new().unwrap(),
        }
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_clw"))
            .current_dir(self.root.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home.path())
            .env("CLW_ENDPOINT", self.server.uri())
            .env("CLW_TENANT", "test")
            .env("CLW_TOKEN", "test-pat")
            .env("CLW_REF_DOMAIN", "user")
            .env("CLW_CACHE_DIR", self.home.path().join("cache"))
            .args(args)
            .output()
            .unwrap()
    }

    fn executions(&self) -> usize {
        std::fs::read(self.root.path().join("executions")).map_or(0, |bytes| bytes.len())
    }

    fn add_orphan(&self) -> String {
        let bytes = b"unreferenced GC control\n".to_vec();
        let hash = Digest::of_bytes(&bytes).to_hex();
        assert!(self
            .state
            .lock()
            .unwrap()
            .cas
            .insert(hash.clone(), bytes)
            .is_none());
        hash
    }
}

fn successful_json(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("one JSON result from the real CLI")
}

async fn assert_record_streams_survive_gc(stored_exit: i32) {
    let fixture = Fixture::new().await;
    // Keep an independent workspace root live. Losing the memo's roots must not
    // accidentally be hidden by prune's separate empty-live-set refusal.
    successful_json(&fixture.cli(&["--json", "snapshot", ".", "--name", "gc-control"]));
    let output = fixture.cli(&[
        "run",
        "--input",
        "input.txt",
        "--",
        "/bin/sh",
        "-c",
        "printf x >> executions; printf memo-out; printf memo-err >&2",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"memo-out");
    assert_eq!(fixture.executions(), 1);

    let (memo_key, stream_hashes) = {
        let mut state = fixture.state.lock().unwrap();
        assert_eq!(state.ac.len(), 2, "one workspace ref and one actual memo");
        let records: Vec<_> = state
            .ac
            .iter()
            .filter_map(|(key, bytes)| {
                clw_run::run_record_blobs(bytes).map(|streams| (key.clone(), streams))
            })
            .collect();
        assert_eq!(records.len(), 1);
        let (key, streams) = &records[0];
        let hashes = vec![streams.0.to_hex(), streams.1.to_hex()];
        assert_ne!(hashes[0], hashes[1], "distinct stdout/stderr controls");
        assert_eq!(state.cas[&hashes[0]], b"memo-out");
        assert_eq!(state.cas[&hashes[1]], b"memo-err");
        let mut record: Value = serde_json::from_slice(&state.ac[key]).unwrap();
        record["exit_code"] = json!(stored_exit);
        state
            .ac
            .insert(key.clone(), serde_json::to_vec(&record).unwrap());
        (key.clone(), hashes)
    };
    // Do not ask the recognition helper to classify the changed record here:
    // reachability must be observed through the real CLI prune path itself.
    let orphan = fixture.add_orphan();
    let (mut expected_cas, expected_ac) = {
        let state = fixture.state.lock().unwrap();
        (state.cas.clone(), state.ac.clone())
    };
    let dry = successful_json(&fixture.cli(&["--json", "prune"]));
    assert_eq!(dry["dry_run"], json!(true));
    assert_eq!(dry["orphans"], json!(1));
    assert_eq!(dry["would_delete"], json!([&orphan]));
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.cas, expected_cas, "dry-run must not mutate CAS");
        assert_eq!(state.ac, expected_ac, "dry-run must not mutate AC");
        assert!(state.requests.iter().all(|(method, _)| method != "DELETE"));
    }

    let swept = successful_json(&fixture.cli(&["--json", "prune", "--yes"]));
    assert_eq!(swept["dry_run"], json!(false));
    assert_eq!(swept["deleted"], json!(1));
    assert!(expected_cas.remove(&orphan).is_some());
    assert_eq!(swept["live"], json!(expected_cas.len()));
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(
            state.cas, expected_cas,
            "only the genuine orphan may disappear"
        );
        assert_eq!(
            state.ac, expected_ac,
            "all original refs and the occupant survive"
        );
        let deletes: Vec<_> = state
            .requests
            .iter()
            .filter(|(method, _)| method == "DELETE")
            .cloned()
            .collect();
        assert_eq!(
            deletes,
            vec![("DELETE".into(), format!("/v1/cas/test/{orphan}"))]
        );
        assert!(state.requests.iter().any(|(method, path)| {
            method == "GET" && path == &format!("/v1/ac/test/{memo_key}")
        }));
    }

    // A new transport/cache observes the remote bytes without invoking `run`
    // again; a fallback child must not regenerate missing streams and mask loss.
    let cold_read_start = fixture.state.lock().unwrap().requests.len();
    let cold = TempDir::new().unwrap();
    let client = HttpClient::new(Config {
        endpoint: fixture.server.uri(),
        tenant: "test".into(),
        token: "test-pat".into(),
        cache_dir: cold.path().join("cache"),
    })
    .unwrap();
    for hash in stream_hashes {
        let digest = Digest::from_hex(&hash).unwrap();
        let bytes = CasTransport::get(&client, &digest).await.unwrap();
        assert_eq!(bytes, expected_cas[&hash]);
        let state = fixture.state.lock().unwrap();
        assert!(state.requests[cold_read_start..]
            .iter()
            .any(|(method, path)| { method == "GET" && path == &format!("/v1/cas/test/{hash}") }));
    }
    assert_eq!(
        fixture.executions(),
        1,
        "GC verification must not rerun the child"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonzero_memo_streams_survive_real_cli_prune() {
    assert_record_streams_survive_gc(42).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_memo_streams_survive_real_cli_prune() {
    assert_record_streams_survive_gc(0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_live_set_still_refuses_prune_without_deleting() {
    let fixture = Fixture::new().await;
    let orphan = fixture.add_orphan();
    let output = fixture.cli(&["--json", "prune", "--yes"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"], json!("empty-live-set-refused"));
    let state = fixture.state.lock().unwrap();
    assert!(state.cas.contains_key(&orphan));
    assert!(state.requests.iter().all(|(method, _)| method != "DELETE"));
}
