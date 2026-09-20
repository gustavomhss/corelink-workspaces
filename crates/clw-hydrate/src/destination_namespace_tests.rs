//! #303: actual filesystem naming and original hydrate/publication paths.
#![cfg(test)]
use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture() -> tempfile::TempDir {
    match std::env::var_os("CLW303_TEST_ROOT") {
        Some(root) => tempfile::tempdir_in(root).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

fn aliases(parent: &Path, a: &str, b: &str) -> bool {
    let probe = tempfile::tempdir_in(parent).unwrap();
    std::fs::write(probe.path().join(a), b"probe").unwrap();
    probe.path().join(b).exists()
}

struct Store {
    root: Digest,
    blobs: BTreeMap<String, Vec<u8>>,
    chunk_gets: AtomicUsize,
}
#[async_trait::async_trait]
impl CasTransport for Store {
    async fn exists(&self, d: &Digest) -> Result<bool> {
        Ok(self.blobs.contains_key(&d.to_hex()))
    }
    async fn get(&self, d: &Digest) -> Result<Vec<u8>> {
        if *d != self.root {
            self.chunk_gets.fetch_add(1, Ordering::Relaxed);
        }
        self.blobs
            .get(&d.to_hex())
            .cloned()
            .ok_or(ClwError::NotFound(*d))
    }
    async fn put(&self, _: &Digest, _: Vec<u8>) -> Result<()> {
        panic!("hydrate cannot publish remote data")
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("hydrate cannot delete remote data")
    }
    async fn list_blobs(&self) -> Result<Vec<Digest>> {
        panic!("unexpected listing")
    }
}
#[async_trait::async_trait]
impl AcTransport for Store {
    async fn get(&self, _: &Digest) -> Result<Option<Vec<u8>>> {
        panic!("root override must skip AC")
    }
    async fn put(&self, _: &Digest, _: Vec<u8>) -> Result<()> {
        panic!("unexpected put")
    }
    async fn delete(&self, _: &Digest) -> Result<()> {
        panic!("unexpected delete")
    }
    async fn list_refs(&self) -> Result<Vec<Digest>> {
        panic!("unexpected listing")
    }
}

fn store(files: &[(&str, &[u8])], dirs: &[&str]) -> Store {
    let mut blobs = BTreeMap::new();
    let mut entries: Vec<Entry> = dirs
        .iter()
        .map(|p| Entry::Dir { path: (*p).into() })
        .collect();
    let mut total_size = 0;
    for &(path, bytes) in files {
        let digest = Digest::of_bytes(bytes);
        blobs.insert(digest.to_hex(), bytes.to_vec());
        total_size += bytes.len() as u64;
        entries.push(Entry::File {
            path: path.into(),
            mode: 0o600,
            size: bytes.len() as u64,
            chunks: vec![clw_types::ChunkRef {
                digest,
                size: bytes.len() as u64,
            }],
        });
    }
    entries.sort_by(|a, b| a.path().cmp(b.path()));
    let manifest = Manifest {
        version: 1,
        total_size,
        entries,
    };
    let bytes = manifest.canonical_bytes().unwrap();
    let root = Digest::of_bytes(&bytes);
    blobs.insert(root.to_hex(), bytes);
    Store {
        root,
        blobs,
        chunk_gets: AtomicUsize::new(0),
    }
}

async fn restore(
    parent: &Path,
    dest: &Path,
    s: &Store,
    concurrency: usize,
) -> Result<HydrateReport> {
    let cache_dir = tempfile::tempdir_in(parent).unwrap();
    let cache = LocalCache::with_budget(cache_dir.path(), 1024 * 1024)?;
    let mut opts = HydrateOptions::from_manifest_digest(s.root);
    opts.concurrency = concurrency;
    hydrate(dest, s, &cache, &opts).await
}

async fn check_files(a: &str, b: &str) {
    for concurrency in [1, 4] {
        let f = fixture();
        let collides = aliases(f.path(), a, b);
        let dest = f.path().join("destination");
        let s = store(&[(a, b"AAA"), (b, b"BBB")], &[]);
        let result = restore(f.path(), &dest, &s, concurrency).await;
        eprintln!("CLW303 files {a:?}/{b:?} aliases={collides} concurrency={concurrency}");
        if collides {
            assert!(
                result.is_err(),
                "distinct source names silently collapsed: {result:?}"
            );
            assert_eq!(
                s.chunk_gets.load(Ordering::Relaxed),
                0,
                "collision must be rejected before content fetch"
            );
            assert_eq!(
                std::fs::read_dir(&dest).unwrap().count(),
                0,
                "probe or output leaked after preflight refusal"
            );
        } else {
            assert_eq!(result.unwrap().files, 2);
            assert_eq!(std::fs::read(dest.join(a)).unwrap(), b"AAA");
            assert_eq!(std::fs::read(dest.join(b)).unwrap(), b"BBB");
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 2);
            use std::os::unix::fs::MetadataExt;
            assert_ne!(
                std::fs::metadata(dest.join(a)).unwrap().ino(),
                std::fs::metadata(dest.join(b)).unwrap().ino()
            );
        }
    }
}

#[tokio::test]
async fn ascii_aliases_are_rejected_before_content_and_distinct_names_survive() {
    check_files("A.txt", "a.txt").await;
    check_files("a.txt", "b.txt").await;
}

#[tokio::test]
async fn normalization_aliases_follow_actual_volume_rules() {
    check_files("\u{00e9}.txt", "e\u{0301}.txt").await;
}

#[tokio::test]
async fn implicit_directory_aliases_are_not_merged() {
    for concurrency in [1, 4] {
        let f = fixture();
        let collides = aliases(f.path(), "Upper", "upper");
        let dest = f.path().join("destination");
        let s = store(&[("Upper/one", b"one"), ("upper/two", b"two")], &[]);
        let result = restore(f.path(), &dest, &s, concurrency).await;
        if collides {
            assert!(result.is_err(), "distinct source directories were merged");
            assert_eq!(s.chunk_gets.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        } else {
            assert_eq!(result.unwrap().files, 2);
            assert_eq!(std::fs::read(dest.join("Upper/one")).unwrap(), b"one");
            assert_eq!(std::fs::read(dest.join("upper/two")).unwrap(), b"two");
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 2);
        }
    }
}

#[tokio::test]
async fn file_directory_aliases_refuse_without_partial_content() {
    let f = fixture();
    let collides = aliases(f.path(), "Upper", "upper");
    for (i, dirs) in [vec![], vec!["Upper"]].into_iter().enumerate() {
        let dest = f.path().join(format!("destination-{i}"));
        let s = store(&[("Upper/child", b"child"), ("upper", b"file")], &dirs);
        let result = restore(f.path(), &dest, &s, 4).await;
        if collides {
            assert!(result.is_err());
            assert_eq!(s.chunk_gets.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        } else {
            assert_eq!(result.unwrap().files, 2);
            assert_eq!(std::fs::read(dest.join("upper")).unwrap(), b"file");
            assert_eq!(std::fs::read(dest.join("Upper/child")).unwrap(), b"child");
        }
    }
}

#[tokio::test]
async fn explicit_directories_and_shared_literal_parents_still_work() {
    let f = fixture();
    let dest = f.path().join("destination");
    let s = store(
        &[("one/a", b"a"), ("one/b", b"b"), ("two/deep/c", b"c")],
        &["empty", "one", "two", "two/deep"],
    );
    assert_eq!(restore(f.path(), &dest, &s, 4).await.unwrap().files, 3);
    for (path, bytes) in [("one/a", b"a"), ("one/b", b"b"), ("two/deep/c", b"c")] {
        assert_eq!(std::fs::read(dest.join(path)).unwrap(), bytes);
    }
    assert_eq!(std::fs::read_dir(dest.join("empty")).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 3);
}

#[tokio::test]
async fn late_publication_collision_preserves_existing_bytes_and_removes_own_temp() {
    let f = fixture();
    let dest = f.path().join("destination");
    std::fs::create_dir(&dest).unwrap();
    let tmp = dest.join(".owned-temp");
    let final_path = dest.join("final");
    let (tx, writer) = spawn_file_writer(tmp.clone(), final_path.clone(), dest.clone(), 0o600);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    tx.send(FileWrite::Chunk(b"new complete bytes".to_vec()))
        .await
        .unwrap();
    tx.send(FileWrite::BeforePublish {
        ready: ready_tx,
        resume: resume_rx,
    })
    .await
    .unwrap();
    tx.send(FileWrite::Finish).await.unwrap();
    drop(tx);
    tokio::time::timeout(std::time::Duration::from_secs(10), ready_rx)
        .await
        .unwrap()
        .unwrap();
    std::fs::write(&final_path, b"existing bytes").unwrap();
    resume_tx.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), writer)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err(), "writer overwrote a late occupant");
    assert_eq!(std::fs::read(&final_path).unwrap(), b"existing bytes");
    assert!(!tmp.exists());
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1);
}

fn assert_collision(result: &Result<HydrateReport>) {
    assert!(
        matches!(result, Err(ClwError::InvalidManifest(message))
        if message.contains("destination namespace collision")),
        "{result:?}"
    );
}

#[tokio::test]
async fn nested_names_use_native_equivalence_and_keep_distinct_object_ids() {
    use std::os::unix::fs::MetadataExt;
    for concurrency in [1, 4] {
        let f = fixture();
        let collides = aliases(f.path(), "A.txt", "a.txt");
        let dest = f.path().join("destination");
        let s = store(
            &[("deep/one/A.txt", b"AAA"), ("deep/one/a.txt", b"BBB")],
            &[],
        );
        let result = restore(f.path(), &dest, &s, concurrency).await;
        if collides {
            assert_collision(&result);
            assert_eq!(s.chunk_gets.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        } else {
            assert_eq!(result.unwrap().files, 2);
            let a = dest.join("deep/one/A.txt");
            let b = dest.join("deep/one/a.txt");
            assert_eq!(std::fs::read(&a).unwrap(), b"AAA");
            assert_eq!(std::fs::read(&b).unwrap(), b"BBB");
            assert_ne!(
                std::fs::metadata(a).unwrap().ino(),
                std::fs::metadata(b).unwrap().ino()
            );
            assert_eq!(std::fs::read_dir(dest.join("deep/one")).unwrap().count(), 2);
        }
    }
}

#[tokio::test]
async fn symlink_names_are_probed_as_names_without_following_targets() {
    for concurrency in [1, 4] {
        let f = fixture();
        let dest = f.path().join("destination");
        let collides = aliases(f.path(), "Link", "link");
        let mut s = store(&[("target", b"content")], &[]);
        let mut manifest = Manifest::from_bytes(s.blobs.get(&s.root.to_hex()).unwrap()).unwrap();
        for path in ["Link", "link"] {
            manifest.entries.push(Entry::Symlink {
                path: path.into(),
                target: "target".into(),
            });
        }
        manifest.entries.sort_by(|a, b| a.path().cmp(b.path()));
        let encoded = manifest.canonical_bytes().unwrap();
        s.root = Digest::of_bytes(&encoded);
        s.blobs.insert(s.root.to_hex(), encoded.clone());
        let result = restore(f.path(), &dest, &s, concurrency).await;
        if collides {
            assert_collision(&result);
            assert_eq!(s.chunk_gets.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        } else {
            assert_eq!(result.unwrap().files, 1);
            for name in ["Link", "link"] {
                assert_eq!(
                    std::fs::read_link(dest.join(name)).unwrap(),
                    PathBuf::from("target")
                );
                assert_eq!(std::fs::read(dest.join(name)).unwrap(), b"content");
            }
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 3);
        }
        assert_eq!(s.blobs.get(&s.root.to_hex()).unwrap(), &encoded);
    }
}

#[tokio::test]
async fn competing_complete_writers_publish_exactly_one_without_overwrite() {
    let f = fixture();
    let dest = f.path().join("destination");
    std::fs::create_dir(&dest).unwrap();
    let final_path = dest.join("final");
    let mut writers = Vec::new();
    let mut releases = Vec::new();
    for (name, bytes) in [
        (".first-temp", b"first".as_slice()),
        (".second-temp", b"second".as_slice()),
    ] {
        let (tx, writer) =
            spawn_file_writer(dest.join(name), final_path.clone(), dest.clone(), 0o600);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        tx.send(FileWrite::Chunk(bytes.to_vec())).await.unwrap();
        tx.send(FileWrite::BeforePublish {
            ready: ready_tx,
            resume: resume_rx,
        })
        .await
        .unwrap();
        tx.send(FileWrite::Finish).await.unwrap();
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_secs(10), ready_rx)
            .await
            .unwrap()
            .unwrap();
        writers.push((writer, bytes));
        releases.push(resume_tx);
    }
    assert!(
        !final_path.exists(),
        "a partial final name must not be exposed"
    );
    for release in releases {
        release.send(()).unwrap();
    }
    let mut successful = 0;
    for (writer, bytes) in writers {
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), writer)
            .await
            .unwrap()
            .unwrap();
        match result {
            Ok(()) => {
                successful += 1;
                assert_eq!(std::fs::read(&final_path).unwrap(), bytes);
            }
            Err(ClwError::Io(error)) => assert_eq!(error.kind(), ErrorKind::AlreadyExists),
            other => panic!("unexpected publication result: {other:?}"),
        }
    }
    assert_eq!(successful, 1);
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1);
}
