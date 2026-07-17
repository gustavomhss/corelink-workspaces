//! Property-based lifecycle E2E — the "every molecule" fidelity net.
//!
//! An ARBITRARY workspace tree (varied names incl. unicode + spaces, 0–2 nested dir
//! levels, empty + multi-byte files, exec bits, an in-tree symlink) must
//! `snapshot -> hydrate` **byte-identical**, with the path set, exec bit, and symlink
//! target all preserved. Fixed-input round-trips already exist (adversarial.rs); this
//! sweeps the SHAPE space so a regression on some odd tree can't hide.
//!
//! Runs against an in-process shared CAS+AC store (no HTTP), so proptest can explore
//! many trees fast enough for the gate.
#![cfg(unix)] // uses std::os::unix::fs (symlink + PermissionsExt), like the sibling tests

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use clw_cache::LocalCache;
use clw_types::{AcTransport, CasTransport, ClwError, Digest, Result};
use proptest::prelude::*;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// In-process shared CAS + AC store. snapshot writes; hydrate reads the same maps.
// ---------------------------------------------------------------------------
#[derive(Clone, Default)]
struct MemTransport {
    cas: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
    ac: Arc<Mutex<HashMap<Digest, Vec<u8>>>>,
}

#[async_trait::async_trait]
impl CasTransport for MemTransport {
    async fn exists(&self, d: &Digest) -> Result<bool> {
        Ok(self.cas.lock().unwrap().contains_key(d))
    }
    async fn get(&self, d: &Digest) -> Result<Vec<u8>> {
        self.cas
            .lock()
            .unwrap()
            .get(d)
            .cloned()
            .ok_or(ClwError::NotFound(*d))
    }
    async fn put(&self, d: &Digest, bytes: Vec<u8>) -> Result<()> {
        self.cas.lock().unwrap().insert(*d, bytes);
        Ok(())
    }
    async fn delete(&self, d: &Digest) -> Result<()> {
        self.cas.lock().unwrap().remove(d);
        Ok(())
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        Ok(self.cas.lock().unwrap().keys().copied().collect())
    }
}

#[async_trait::async_trait]
impl AcTransport for MemTransport {
    async fn get(&self, k: &Digest) -> Result<Option<Vec<u8>>> {
        Ok(self.ac.lock().unwrap().get(k).cloned())
    }
    async fn put(&self, k: &Digest, v: Vec<u8>) -> Result<()> {
        self.ac.lock().unwrap().insert(*k, v);
        Ok(())
    }
    async fn delete(&self, k: &Digest) -> Result<()> {
        self.ac.lock().unwrap().remove(k);
        Ok(())
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        Ok(self.ac.lock().unwrap().keys().copied().collect())
    }
}

// ---------------------------------------------------------------------------
// Tree generation
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
struct FileSpec {
    content: Vec<u8>,
    exec: bool,
}

/// A single path component. LOWERCASE-only on purpose: the gate's `cargo test`
/// runs on a case-INSENSITIVE macOS filesystem, where `fA` and `fa` would collide
/// and make the round-trip set flaky. `f`-prefixed so a component is never ""/"."/"..".
fn name_component() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9_-]{1,10}",            // plain ascii
        "[a-z0-9]{1,6}[áéíçü会]{1,3}", // unicode-bearing
        "[a-z0-9]{1,4} [a-z0-9]{1,4}", // embedded space
    ]
    .prop_map(|s| format!("f{s}"))
}

/// 0–2 dir levels + a filename, joined with '/'.
fn rel_path() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(name_component(), 0..3),
        name_component(),
    )
        .prop_map(|(dirs, file)| {
            let mut parts = dirs;
            parts.push(file);
            parts.join("/")
        })
}

fn tree_strategy() -> impl Strategy<Value = BTreeMap<String, FileSpec>> {
    prop::collection::btree_map(
        rel_path(),
        (prop::collection::vec(any::<u8>(), 0..4096), any::<bool>())
            .prop_map(|(content, exec)| FileSpec { content, exec }),
        0..24,
    )
}

/// True if path `a` is an ancestor DIRECTORY of path `b` (so one wants a dir where
/// the other wants a file — a filesystem collision we must not materialize).
fn is_ancestor(a: &str, b: &str) -> bool {
    b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
}

proptest! {
    // 32 cases keeps the per-PR gate cost ~21s (mean ~12 files/case, so coverage is
    // barely affected vs 64); the stress lane cranks PROPTEST_CASES far higher.
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Any tree snapshots -> hydrates byte-identical, structure + exec-bit + symlink preserved.
    #[test]
    fn arbitrary_tree_roundtrips_byte_identical(gen in tree_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(roundtrip_one(gen));
    }
}

async fn roundtrip_one(gen: BTreeMap<String, FileSpec>) {
    let src = TempDir::new().unwrap();

    // Materialize a COLLISION-FREE subset: skip any path that is an ancestor dir of,
    // or a descendant of, an already-written path (can't be both a dir and a file).
    // We then assert against exactly what we wrote.
    let mut written: BTreeMap<String, FileSpec> = BTreeMap::new();
    'next: for (rel, spec) in gen {
        for existing in written.keys() {
            if existing == &rel || is_ancestor(existing, &rel) || is_ancestor(&rel, existing) {
                continue 'next;
            }
        }
        let abs = src.path().join(&rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&abs, &spec.content).unwrap();
        if spec.exec {
            let mut p = std::fs::metadata(&abs).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&abs, p).unwrap();
        }
        written.insert(rel, spec);
    }

    // Add one in-tree symlink pointing at the first file (name is un-generatable —
    // components are f-prefixed and ≤ this length can't be produced by the strategy).
    let symlink: Option<(String, String)> = written.keys().next().map(|target| {
        let link = "flink_probe_0".to_string();
        std::os::unix::fs::symlink(target, src.path().join(&link)).unwrap();
        (link, target.clone())
    });

    // snapshot -> hydrate through one shared in-memory store.
    let transport = MemTransport::default();
    let cache1 = TempDir::new().unwrap();
    let lc1 = LocalCache::new(cache1.path()).unwrap();
    clw_snapshot::snapshot(
        src.path(),
        &transport,
        &lc1,
        &clw_snapshot::SnapshotOptions::new("prop-ws"),
    )
    .await
    .expect("snapshot must succeed on an arbitrary valid tree");

    let dest = TempDir::new().unwrap();
    let out = dest.path().join("hydrated");
    let cache2 = TempDir::new().unwrap();
    let lc2 = LocalCache::new(cache2.path()).unwrap();
    clw_hydrate::hydrate(
        &out,
        &transport,
        &lc2,
        &clw_hydrate::HydrateOptions::new("prop-ws"),
    )
    .await
    .expect("hydrate must succeed");

    // Every file: byte-identical content + preserved exec bit.
    for (rel, spec) in &written {
        let got = std::fs::read(out.join(rel))
            .unwrap_or_else(|e| panic!("hydrated file '{rel}' missing/unreadable: {e}"));
        assert_eq!(
            &got, &spec.content,
            "content of '{rel}' must round-trip byte-identical"
        );
        let mode = std::fs::symlink_metadata(out.join(rel))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o100 != 0,
            spec.exec,
            "exec bit of '{rel}' must be preserved (mode {mode:o})"
        );
    }

    // Symlink target preserved verbatim.
    if let Some((link, target)) = &symlink {
        let got = std::fs::read_link(out.join(link)).unwrap();
        assert_eq!(
            got.to_str().unwrap(),
            target,
            "symlink target must be preserved"
        );
    }

    // No extra, no missing: the hydrated regular-file set == what we wrote.
    let mut hydrated_files: Vec<String> = Vec::new();
    collect_files(&out, &out, &mut hydrated_files);
    hydrated_files.sort();
    let mut expected: Vec<String> = written.keys().cloned().collect();
    if let Some((link, _)) = &symlink {
        expected.push(link.clone());
    }
    expected.sort();
    assert_eq!(
        hydrated_files, expected,
        "hydrated path set must equal the source set"
    );
}

/// Recursively collect regular files + symlinks under `dir`, as paths relative to `root`.
fn collect_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.file_type().is_dir() {
            collect_files(root, &path, out);
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}
