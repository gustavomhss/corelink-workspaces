//! Versioned, additive JSON-output contract checks.
//!
//! The golden records sorted top-level keys by selector and outcome. Values are
//! produced by the same builders that production dispatch prints, rather than by
//! test-only `json!` copies. This first, deliberately narrow G2 slice covers the
//! two selectors changed by WP43 and every JSON success/error outcome those
//! selectors can emit. The broader command-family inventory remains separate to
//! avoid a high-risk refactor of unrelated command output paths.

use std::collections::BTreeMap;

use clw_snapshot::SnapshotReport;
use clw_types::{Digest, RefRecord};
use serde_json::Value;

use super::{ls, snapshot};

type Golden = BTreeMap<&'static str, BTreeMap<&'static str, Vec<String>>>;

fn top_level_keys(value: Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .expect("JSON contract emitters must produce an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

fn sample_ref_record() -> RefRecord {
    RefRecord {
        name: "workspace".to_string(),
        root: Digest::of_bytes(b"root"),
        manifest_size: 7,
        created_at_unix: 1_700_000_000,
        parent: None,
        tool_version: "test".to_string(),
    }
}

fn json_contract_v1_actual() -> Golden {
    let record = sample_ref_record();
    let snapshot_report = SnapshotReport {
        root: Digest::of_bytes(b"snapshot"),
        files: 2,
        bytes_total: 17,
        chunks_total: 1,
        chunks_uploaded: 1,
        secret_warnings: vec![".env".to_string()],
        skipped_external_symlinks: vec!["outside -> ../outside".to_string()],
        unchanged: false,
    };
    let mut selectors = BTreeMap::new();
    selectors.insert(
        "ls --all",
        BTreeMap::from([
            (
                "capability-unavailable",
                top_level_keys(ls::build_capability_unavailable_json("D-7 unavailable")),
            ),
            (
                "success",
                top_level_keys(ls::build_all_json(
                    "tenant",
                    std::slice::from_ref(&record),
                    &[("bad-key".to_string(), "bad record".to_string())],
                    1,
                    1,
                )),
            ),
        ]),
    );
    selectors.insert(
        "ls --name",
        BTreeMap::from([(
            "success",
            top_level_keys(ls::build_one_json(&record, 2, 17)),
        )]),
    );
    selectors.insert(
        "snapshot",
        BTreeMap::from([(
            "success",
            top_level_keys(snapshot::build_success_json("workspace", &snapshot_report)),
        )]),
    );
    selectors
}

#[test]
fn json_contract_v1_top_level_keys() {
    let expected: Golden = BTreeMap::from([
        (
            "ls --all",
            BTreeMap::from([
                (
                    "capability-unavailable",
                    vec!["error".to_string(), "note".to_string()],
                ),
                (
                    "success",
                    [
                        "count",
                        "errors",
                        "foreign_ac_entries",
                        "refs",
                        "run_cache_entries",
                        "skipped",
                        "tenant",
                    ]
                    .map(str::to_string)
                    .to_vec(),
                ),
            ]),
        ),
        (
            "ls --name",
            BTreeMap::from([(
                "success",
                [
                    "bytes",
                    "bytes_total",
                    "created_at_unix",
                    "files",
                    "name",
                    "parent",
                    "root",
                ]
                .map(str::to_string)
                .to_vec(),
            )]),
        ),
        (
            "snapshot",
            BTreeMap::from([(
                "success",
                [
                    "bytes_total",
                    "chunks_total",
                    "chunks_uploaded",
                    "files",
                    "name",
                    "root",
                    "secret_warnings",
                    "skipped_external_symlinks",
                    "unchanged",
                ]
                .map(str::to_string)
                .to_vec(),
            )]),
        ),
    ]);

    assert_eq!(json_contract_v1_actual(), expected);
}

#[test]
fn snapshot_json_secret_warnings_are_present_when_empty() {
    let report = SnapshotReport {
        root: Digest::of_bytes(b"clean"),
        files: 0,
        bytes_total: 0,
        chunks_total: 0,
        chunks_uploaded: 0,
        secret_warnings: Vec::new(),
        skipped_external_symlinks: Vec::new(),
        unchanged: true,
    };

    assert_eq!(
        snapshot::build_success_json("clean", &report)["secret_warnings"],
        serde_json::json!([]),
    );
}
