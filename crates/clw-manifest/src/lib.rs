//! clw-manifest — ignore-aware tree walk + manifest diff.
//!
//! Respects `.gitignore` and `.clwignore`, hidden files included, `.git/`
//! always skipped, symlinks not followed, empty directories emitted.

use std::collections::HashMap;
use std::path::Path;

use clw_types::{ClwError, Entry, Manifest, Result};
use ignore::WalkBuilder;

// ---------------------------------------------------------------------------
// Walk output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkedFile {
    pub abs: std::path::PathBuf,
    pub rel: String,
    pub mode: u32,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkedEntry {
    File(WalkedFile),
    Symlink {
        rel: String,
        target: String,
    },
    EmptyDir {
        rel: String,
    },
    /// A symlink whose TARGET is un-representable in the manifest — absolute,
    /// or a `..`-escape that leaves the workspace root. Its target is OUTSIDE the
    /// snapshot, so it cannot be meaningfully restored anyway. Recorded here (not
    /// as a real `Symlink`, and NOT an error) so `snapshot` can SKIP it with a
    /// warning instead of hard-failing the whole tree. The manifest therefore
    /// never contains an external/escaping symlink — containment stays strict.
    SkippedExternalSymlink {
        rel: String,
        target: String,
    },
}

impl WalkedEntry {
    fn rel(&self) -> &str {
        match self {
            WalkedEntry::File(f) => &f.rel,
            WalkedEntry::Symlink { rel, .. } => rel,
            WalkedEntry::EmptyDir { rel } => rel,
            WalkedEntry::SkippedExternalSymlink { rel, .. } => rel,
        }
    }
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

pub fn walk(root: &Path) -> Result<Vec<WalkedEntry>> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .add_custom_ignore_filename(".clwignore")
        .follow_links(false)
        .require_git(false)
        .build();

    // Collect entries; we'll track which directories contain children.
    let mut entries: Vec<WalkedEntry> = Vec::new();
    // Set of directory rel paths that have at least one child entry.
    let mut dirs_with_children: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    // Candidate empty dirs (rel → abs).
    let mut candidate_dirs: Vec<(String, std::path::PathBuf)> = Vec::new();

    for result in walker {
        let dent = result.map_err(|e| ClwError::Io(std::io::Error::other(e.to_string())))?;
        let abs = dent.path().to_path_buf();

        // Skip the root itself.
        if abs == root {
            continue;
        }

        // Build rel path.
        let rel_raw = abs
            .strip_prefix(root)
            .map_err(|_| ClwError::InvalidManifest("path not under root".to_string()))?;

        // Check for .git directory at any level — skip the .git dir entry
        // and everything inside it.
        {
            let mut components = rel_raw.components();
            if let Some(first) = components.next() {
                if first.as_os_str() == ".git" {
                    continue;
                }
            }
        }
        // Also skip if any component is ".git" (nested, shouldn't happen but be safe).
        if rel_raw.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }

        // Convert rel to UTF-8 string with '/' separators.
        let rel: String = rel_raw
            .to_str()
            .ok_or_else(|| {
                ClwError::InvalidManifest(format!("non-utf8 path: {}", rel_raw.to_string_lossy()))
            })?
            .replace(std::path::MAIN_SEPARATOR, "/");

        // Mark parent directories as having children.
        {
            let mut parent = rel_raw.parent();
            while let Some(p) = parent {
                if p == Path::new("") {
                    break;
                }
                let p_str = p
                    .to_str()
                    .ok_or_else(|| {
                        ClwError::InvalidManifest(format!("non-utf8 path: {}", p.to_string_lossy()))
                    })?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                dirs_with_children.insert(p_str);
                parent = p.parent();
            }
        }

        let file_type = dent.file_type().ok_or_else(|| {
            ClwError::InvalidManifest(format!("cannot determine file type for {rel}"))
        })?;

        if file_type.is_symlink() {
            let link_target = std::fs::read_link(&abs)?;
            // Normalize separators ('\\' → '/') so a Windows-produced relative
            // target is portable, then validate at the PRODUCER. Without this a
            // target that can't pass consumer-side validation (backslash,
            // absolute, `..`-escape, colon) would publish a manifest no client
            // can hydrate; failing fast here surfaces an actionable error at
            // snapshot time instead.
            let target = link_target
                .to_str()
                .ok_or_else(|| {
                    ClwError::InvalidManifest(format!(
                        "non-utf8 path: {}",
                        link_target.to_string_lossy()
                    ))
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            // FIX 3: validate the entry's OWN rel path too (not just the target),
            // so an unrepresentable symlink path fails fast at walk time. A bad LINK
            // PATH (its own filename) is genuinely unrepresentable and rare — still a
            // hard error.
            clw_types::validate_rel_path(&rel)?;
            // Resolve the target against the LINK's location and allow `..` that
            // stays inside the workspace (real projects have e.g. `bin/x -> ../src/x`).
            // An escaping/absolute/otherwise-unrepresentable TARGET points OUTSIDE the
            // workspace, so its bytes aren't in the snapshot and it cannot be restored
            // anyway — record it as SKIPPED (not an error, and NOT a real Symlink) so
            // the rest of the tree snapshots fine. The validator stays STRICT: a target
            // it rejects is NEVER emitted as a `Symlink`, so the manifest can never
            // carry an external/escaping symlink.
            match clw_types::validate_symlink_target_contained(&rel, &target) {
                Ok(()) => entries.push(WalkedEntry::Symlink { rel, target }),
                Err(_) => entries.push(WalkedEntry::SkippedExternalSymlink { rel, target }),
            }
        } else if file_type.is_dir() {
            // FIX 3: validate Dir entry paths at walk time — the SAME point symlink
            // targets are validated — so a path `validate_rel_path` will reject
            // (e.g. a filename containing a backslash, legal on unix) fails FAST
            // with a clear per-entry error, instead of surviving until
            // `manifest.canonical_bytes()` AFTER every chunk has been uploaded.
            // `validate_rel_path` is the authority; this only moves the check
            // earlier — it does not change what is accepted or rejected.
            clw_types::validate_rel_path(&rel)?;
            // Record as a candidate empty dir; we'll filter after collecting all entries.
            candidate_dirs.push((rel, abs));
        } else if file_type.is_file() {
            // TOCTOU NOTE (audit Low, accepted for v0.1): this branch classifies
            // `abs` as a regular file using the walker's `follow_links(false)`
            // file type, but the snapshot read site (clw-snapshot, `std::fs::read`
            // / `File::open`) opens it with symlink following. A concurrent hostile
            // writer on a SHARED checkout could swap `abs` for a symlink between this
            // classification and that open, causing snapshot to ingest out-of-workspace
            // bytes. This is OUT OF the v0.1 threat model: `clw` is a single-operator
            // local tool and a concurrent hostile writer to one's own working tree is
            // not modeled. Hardening (cross-platform O_NOFOLLOW open + fstat-after-open
            // to confirm the opened inode is the regular file we classified) is tracked
            // separately; see the matching note in clw-snapshot at the read site.
            let meta = std::fs::metadata(&abs)?;
            // On unix, read the real permission bits. On Windows there is no
            // unix mode concept, so we record a constant default of 0o644 for
            // every regular file (symlink DETECTION below stays cross-platform).
            //
            // Mask to 0o777, NOT 0o7777: hydrate deliberately strips
            // setuid/setgid/sticky from untrusted manifests (security), so
            // recording those high bits here is both unusable AND breaks
            // snapshot→hydrate→snapshot digest idempotency. Masking them off at
            // walk time keeps the round-trip stable and matches what hydrate writes.
            #[cfg(unix)]
            let mode = meta.permissions().mode() & 0o777;
            #[cfg(windows)]
            let mode = 0o644;
            let size = meta.len();
            // FIX 3: validate the File entry's rel path at walk time (see the Dir
            // branch above) so a path canonical_bytes() would later reject fails
            // FAST — BEFORE this file is chunked and uploaded — naming the file.
            clw_types::validate_rel_path(&rel)?;
            entries.push(WalkedEntry::File(WalkedFile {
                abs,
                rel,
                mode,
                size,
            }));
        }
        // Other file types (device, etc.) are ignored.
    }

    // Add truly empty directories.
    for (rel, _abs) in candidate_dirs {
        if !dirs_with_children.contains(&rel) {
            entries.push(WalkedEntry::EmptyDir { rel });
        }
    }

    // Sort by rel path (byte order).
    entries.sort_by(|a, b| a.rel().cmp(b.rel()));

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Manifest diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

pub fn diff(old: &Manifest, new: &Manifest) -> ManifestDiff {
    let old_map: HashMap<&str, &Entry> = old.entries.iter().map(|e| (e.path(), e)).collect();
    let new_map: HashMap<&str, &Entry> = new.entries.iter().map(|e| (e.path(), e)).collect();

    let mut added: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut changed: Vec<String> = Vec::new();

    for (path, new_entry) in &new_map {
        if let Some(old_entry) = old_map.get(path) {
            if entries_changed(old_entry, new_entry) {
                changed.push(path.to_string());
            }
        } else {
            added.push(path.to_string());
        }
    }

    for path in old_map.keys() {
        if !new_map.contains_key(path) {
            removed.push(path.to_string());
        }
    }

    added.sort();
    removed.sort();
    changed.sort();

    ManifestDiff {
        added,
        removed,
        changed,
    }
}

fn entries_changed(old: &Entry, new: &Entry) -> bool {
    match (old, new) {
        (
            Entry::File {
                mode: old_mode,
                size: old_size,
                chunks: old_chunks,
                ..
            },
            Entry::File {
                mode: new_mode,
                size: new_size,
                chunks: new_chunks,
                ..
            },
        ) =>
        // Mode is part of the manifest (masked `& 0o777` at walk time, the SAME
        // way both the remote snapshot and the local re-walk mask it), so a
        // mode-only change — `chmod +x` with identical content — DOES change the
        // manifest. `status` must not claim CLEAN when a re-snapshot would
        // differ, so the mode is compared here alongside size/chunks. The values
        // are already masked identically on both sides, so the comparison agrees
        // with what `snapshot` would produce.
        {
            old_mode != new_mode || old_size != new_size || old_chunks != new_chunks
        }
        (
            Entry::Symlink {
                target: old_target, ..
            },
            Entry::Symlink {
                target: new_target, ..
            },
        ) => old_target != new_target,
        // Dir entries carry no content other than path (same path = same entry).
        (Entry::Dir { .. }, Entry::Dir { .. }) => false,
        // Same path but different entry kind = changed.
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clw_types::{ChunkRef, Digest, Entry, Manifest, MANIFEST_VERSION};
    use std::fs;
    use tempfile::TempDir;

    fn make_manifest(entries: Vec<Entry>) -> Manifest {
        let total_size = entries
            .iter()
            .map(|e| match e {
                Entry::File { size, .. } => *size,
                _ => 0,
            })
            .sum();
        Manifest {
            version: MANIFEST_VERSION,
            total_size,
            entries,
        }
    }

    // -----------------------------------------------------------------------
    // walk tests
    // -----------------------------------------------------------------------

    #[test]
    fn walk_simple_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("hello.txt"), b"hello").unwrap();

        let entries = walk(root).unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0] {
            WalkedEntry::File(f) => {
                assert_eq!(f.rel, "hello.txt");
                assert_eq!(f.size, 5);
            }
            other => panic!("expected File, got {other:?}"),
        }
    }

    #[test]
    fn walk_gitignore_honored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(root.join("ignored.txt"), b"should not appear").unwrap();
        fs::write(root.join("kept.txt"), b"keep").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                WalkedEntry::File(f) => f.rel.as_str(),
                WalkedEntry::Symlink { rel, .. } => rel.as_str(),
                WalkedEntry::EmptyDir { rel } => rel.as_str(),
                WalkedEntry::SkippedExternalSymlink { rel, .. } => rel.as_str(),
            })
            .collect();

        assert!(rels.contains(&"kept.txt"), "kept.txt missing: {rels:?}");
        assert!(
            !rels.contains(&"ignored.txt"),
            "ignored.txt should be excluded: {rels:?}"
        );
        // .gitignore itself is not ignored by default
    }

    #[test]
    fn walk_clwignore_honored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".clwignore"), "secret.txt\n").unwrap();
        fs::write(root.join("secret.txt"), b"secret").unwrap();
        fs::write(root.join("visible.txt"), b"visible").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                WalkedEntry::File(f) => f.rel.as_str(),
                WalkedEntry::Symlink { rel, .. } => rel.as_str(),
                WalkedEntry::EmptyDir { rel } => rel.as_str(),
                WalkedEntry::SkippedExternalSymlink { rel, .. } => rel.as_str(),
            })
            .collect();

        assert!(
            rels.contains(&"visible.txt"),
            "visible.txt missing: {rels:?}"
        );
        assert!(
            !rels.contains(&"secret.txt"),
            "secret.txt should be excluded: {rels:?}"
        );
    }

    #[test]
    fn walk_git_dir_skipped() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git").join("HEAD"), b"ref: refs/heads/main").unwrap();
        fs::write(root.join("real.txt"), b"real").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                WalkedEntry::File(f) => f.rel.as_str(),
                WalkedEntry::Symlink { rel, .. } => rel.as_str(),
                WalkedEntry::EmptyDir { rel } => rel.as_str(),
                WalkedEntry::SkippedExternalSymlink { rel, .. } => rel.as_str(),
            })
            .collect();

        assert!(rels.contains(&"real.txt"), "real.txt missing: {rels:?}");
        assert!(
            !rels.iter().any(|r| r.starts_with(".git")),
            ".git entries should be absent: {rels:?}"
        );
    }

    #[test]
    fn walk_hidden_files_included() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".hidden"), b"hidden content").unwrap();
        fs::write(root.join("normal.txt"), b"normal").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                WalkedEntry::File(f) => f.rel.as_str(),
                WalkedEntry::Symlink { rel, .. } => rel.as_str(),
                WalkedEntry::EmptyDir { rel } => rel.as_str(),
                WalkedEntry::SkippedExternalSymlink { rel, .. } => rel.as_str(),
            })
            .collect();

        assert!(rels.contains(&".hidden"), ".hidden missing: {rels:?}");
        assert!(rels.contains(&"normal.txt"), "normal.txt missing: {rels:?}");
    }

    #[test]
    fn walk_sorted_output() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("z.txt"), b"z").unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join("m.txt"), b"m").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                WalkedEntry::File(f) => f.rel.as_str(),
                _ => "",
            })
            .collect();

        assert_eq!(rels, vec!["a.txt", "m.txt", "z.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn walk_symlink_captured() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("target.txt"), b"content").unwrap();
        std::os::unix::fs::symlink("target.txt", root.join("link.txt")).unwrap();

        let entries = walk(root).unwrap();
        let symlink = entries
            .iter()
            .find(|e| matches!(e, WalkedEntry::Symlink { rel, .. } if rel == "link.txt"));
        assert!(symlink.is_some(), "symlink not found: {entries:?}");
        if let Some(WalkedEntry::Symlink { rel, target }) = symlink {
            assert_eq!(rel, "link.txt");
            assert_eq!(target, "target.txt");
        }
    }

    // REGRESSION GUARD — special Unix filesystem objects have no manifest
    // representation. They must be ignored rather than treated as regular
    // files (which could block or capture process-local socket state).
    #[cfg(unix)]
    #[test]
    fn walk_ignores_fifo_and_unix_socket() {
        use std::os::unix::fs::FileTypeExt;
        use std::os::unix::net::UnixListener;
        use std::process::Command;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("regular.txt"), b"content").unwrap();

        let fifo = root.join("pipe");
        let status = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo must be available on Unix test hosts");
        assert!(status.success(), "mkfifo failed with {status}");
        assert!(
            fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo(),
            "fixture must be a FIFO"
        );

        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).expect("create Unix socket fixture");
        assert!(
            fs::symlink_metadata(&socket)
                .unwrap()
                .file_type()
                .is_socket(),
            "fixture must be a Unix socket"
        );

        let entries = walk(root).expect("special files must not fail the walk");
        let rels: Vec<&str> = entries.iter().map(WalkedEntry::rel).collect();
        assert_eq!(
            rels,
            vec!["regular.txt"],
            "FIFO and Unix socket must not produce manifest entries: {entries:?}"
        );

        drop(listener);
    }

    // Option C — a symlink whose TARGET is ABSOLUTE (escapes the workspace) must
    // NOT hard-fail the walk: it is recorded as `SkippedExternalSymlink` while the
    // rest of the tree (a normal file) walks fine. The manifest never carries it.
    // RED-on-revert: restoring the `validate_symlink_target_contained(..)?` `?`
    // turns this into an `Err`, failing the assertion (and the snapshot test below).
    #[cfg(unix)]
    #[test]
    fn walk_skips_external_symlink_keeps_rest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("real.txt"), b"content").unwrap();
        // Absolute target → escapes the workspace, un-representable.
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();

        let entries = walk(root).expect("an external symlink must NOT fail the whole walk");

        // The normal file is still present as a File entry.
        assert!(
            entries
                .iter()
                .any(|e| matches!(e, WalkedEntry::File(f) if f.rel == "real.txt")),
            "the normal file must still be walked: {entries:?}"
        );
        // The escaping symlink is recorded as SKIPPED — NOT as a real Symlink.
        assert!(
            entries.iter().any(|e| matches!(
                e,
                WalkedEntry::SkippedExternalSymlink { rel, target }
                    if rel == "link" && target == "/etc/passwd"
            )),
            "the external symlink must be recorded as SkippedExternalSymlink: {entries:?}"
        );
        assert!(
            !entries
                .iter()
                .any(|e| matches!(e, WalkedEntry::Symlink { rel, .. } if rel == "link")),
            "an escaping symlink must NEVER be emitted as a real Symlink: {entries:?}"
        );
    }

    // REGRESSION GUARD — an INTRA-workspace relative symlink (`bin/x -> ../src/x`)
    // stays inside the root, so it is STILL a normal `Symlink` entry, NOT skipped.
    // This proves the skip fires only for escaping/absolute targets.
    #[cfg(unix)]
    #[test]
    fn walk_intra_workspace_symlink_still_recorded() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("bin")).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src").join("x"), b"real").unwrap();
        // `bin/x -> ../src/x` resolves to `src/x`, which stays inside the workspace.
        std::os::unix::fs::symlink("../src/x", root.join("bin").join("x")).unwrap();

        let entries = walk(root).unwrap();
        assert!(
            entries.iter().any(|e| matches!(
                e,
                WalkedEntry::Symlink { rel, target }
                    if rel == "bin/x" && target == "../src/x"
            )),
            "an intra-workspace symlink must stay a normal Symlink entry: {entries:?}"
        );
        assert!(
            !entries.iter().any(
                |e| matches!(e, WalkedEntry::SkippedExternalSymlink { rel, .. } if rel == "bin/x")
            ),
            "an intra-workspace symlink must NOT be skipped: {entries:?}"
        );
    }

    #[test]
    fn walk_empty_dir_emitted_non_empty_not() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Empty dir
        fs::create_dir(root.join("empty_dir")).unwrap();
        // Non-empty dir (has a file)
        fs::create_dir(root.join("non_empty_dir")).unwrap();
        fs::write(root.join("non_empty_dir").join("file.txt"), b"data").unwrap();

        let entries = walk(root).unwrap();
        let has_empty_dir = entries
            .iter()
            .any(|e| matches!(e, WalkedEntry::EmptyDir { rel } if rel == "empty_dir"));
        let has_non_empty_dir = entries
            .iter()
            .any(|e| matches!(e, WalkedEntry::EmptyDir { rel } if rel == "non_empty_dir"));

        assert!(has_empty_dir, "empty_dir not emitted: {entries:?}");
        assert!(
            !has_non_empty_dir,
            "non_empty_dir should not be emitted as EmptyDir: {entries:?}"
        );
    }

    // FIX 3 (unix) — a file whose name contains a backslash (legal on unix, but
    // rejected by `validate_rel_path` for portability) must fail at WALK time,
    // BEFORE any chunk is uploaded, with an error that names the offending path.
    #[cfg(unix)]
    #[test]
    fn walk_rejects_backslash_named_file_at_walk_time() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // `a\b.txt` — a single unix filename literally containing a backslash.
        let bad_name = "a\\b.txt";
        fs::write(root.join(bad_name), b"data").unwrap();

        let err = walk(root).expect_err("walk must reject a backslash-named file");
        match err {
            ClwError::InvalidManifest(msg) => {
                assert!(
                    msg.contains("backslash"),
                    "error should explain the backslash rejection, got: {msg}"
                );
                assert!(
                    msg.contains(bad_name),
                    "error should name the offending file, got: {msg}"
                );
            }
            other => panic!("expected InvalidManifest, got: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn walk_mode_captured() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let exec_path = root.join("exec.sh");
        let normal_path = root.join("normal.txt");
        fs::write(&exec_path, b"#!/bin/sh").unwrap();
        fs::write(&normal_path, b"text").unwrap();

        let mut perms = fs::metadata(&exec_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&exec_path, perms).unwrap();

        let mut perms2 = fs::metadata(&normal_path).unwrap().permissions();
        perms2.set_mode(0o644);
        fs::set_permissions(&normal_path, perms2).unwrap();

        let entries = walk(root).unwrap();
        let exec_entry = entries
            .iter()
            .find(|e| matches!(e, WalkedEntry::File(f) if f.rel == "exec.sh"));
        let normal_entry = entries
            .iter()
            .find(|e| matches!(e, WalkedEntry::File(f) if f.rel == "normal.txt"));

        if let Some(WalkedEntry::File(f)) = exec_entry {
            assert_eq!(f.mode, 0o755, "exec mode mismatch");
        } else {
            panic!("exec.sh not found");
        }
        if let Some(WalkedEntry::File(f)) = normal_entry {
            assert_eq!(f.mode, 0o644, "normal mode mismatch");
        } else {
            panic!("normal.txt not found");
        }
    }

    // A non-UTF8 filename (a raw 0xFF byte, legal on unix) is un-representable in
    // the '/'-separated UTF-8 manifest path space, so the walk must take the
    // `InvalidManifest("non-utf8 path…")` branch rather than lossily transcode it.
    #[cfg(unix)]
    #[test]
    fn walk_non_utf8_filename_is_invalid_manifest() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // `bad\xffname.txt` — a single unix filename with an invalid-UTF-8 byte.
        let bad = OsStr::from_bytes(b"bad\xffname.txt");
        // Some filesystems (APFS/HFS+ on macOS) enforce valid-UTF-8 filenames and
        // reject this create with EILSEQ. Where the name can't be hosted, the walk
        // branch is untestable HERE — skip cleanly. It is still exercised on ext4
        // (the github-hosted linux coverage/fuzz lanes), where the create succeeds.
        if fs::write(root.join(bad), b"data").is_err() {
            return;
        }

        let err = walk(root).expect_err("a non-utf8 filename must fail the walk");
        match err {
            ClwError::InvalidManifest(msg) => assert!(
                msg.contains("non-utf8 path"),
                "error must name the non-utf8-path branch, got: {msg}"
            ),
            other => panic!("expected InvalidManifest, got: {other:?}"),
        }
    }

    // Alternate ignore sources beyond a root `.gitignore`/`.clwignore` are honored
    // by the walk: (a) a root `.ignore` file, (b) a NESTED (subdirectory)
    // `.gitignore`, and (c) `.git/info/exclude`. Only-root-`.gitignore` was tested.
    #[test]
    fn walk_honors_dot_ignore_nested_gitignore_and_git_exclude() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // (a) root `.ignore` (an alternate ignore source, not `.gitignore`).
        fs::write(root.join(".ignore"), "ign_by_dotignore.txt\n").unwrap();
        fs::write(root.join("ign_by_dotignore.txt"), b"x").unwrap();

        // (b) a NESTED `.gitignore` inside `sub/` ignoring a sibling file.
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join(".gitignore"), "ign_nested.txt\n").unwrap();
        fs::write(root.join("sub").join("ign_nested.txt"), b"x").unwrap();
        fs::write(root.join("sub").join("kept_nested.txt"), b"keep").unwrap();

        // (c) `.git/info/exclude` (git's per-repo local exclude file).
        fs::create_dir_all(root.join(".git").join("info")).unwrap();
        fs::write(
            root.join(".git").join("info").join("exclude"),
            "ign_by_gitexclude.txt\n",
        )
        .unwrap();
        fs::write(root.join("ign_by_gitexclude.txt"), b"x").unwrap();

        // A plain kept file at the root.
        fs::write(root.join("kept.txt"), b"keep").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries.iter().map(|e| e.rel()).collect();

        // Kept files survive.
        assert!(rels.contains(&"kept.txt"), "kept.txt missing: {rels:?}");
        assert!(
            rels.contains(&"sub/kept_nested.txt"),
            "sub/kept_nested.txt missing: {rels:?}"
        );
        // Each alternate ignore source excludes its target.
        assert!(
            !rels.contains(&"ign_by_dotignore.txt"),
            "root .ignore must exclude ign_by_dotignore.txt: {rels:?}"
        );
        assert!(
            !rels.contains(&"sub/ign_nested.txt"),
            "a nested .gitignore must exclude sub/ign_nested.txt: {rels:?}"
        );
        assert!(
            !rels.contains(&"ign_by_gitexclude.txt"),
            ".git/info/exclude must exclude ign_by_gitexclude.txt: {rels:?}"
        );
    }

    // `.git/` exclusion is UNCONDITIONAL and applies at ANY depth: a `.git`
    // directory NESTED under a subdirectory (submodule-shaped) is skipped whole,
    // regardless of ignore files. (Root `.git` is covered by `walk_git_dir_skipped`.)
    #[test]
    fn walk_nested_git_dir_skipped() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("sub").join(".git")).unwrap();
        fs::write(
            root.join("sub").join(".git").join("HEAD"),
            b"ref: refs/heads/main",
        )
        .unwrap();
        fs::write(root.join("sub").join(".git").join("config"), b"[core]").unwrap();
        fs::write(root.join("sub").join("real.txt"), b"real").unwrap();

        let entries = walk(root).unwrap();
        let rels: Vec<&str> = entries.iter().map(|e| e.rel()).collect();

        assert!(
            rels.contains(&"sub/real.txt"),
            "sub/real.txt missing: {rels:?}"
        );
        assert!(
            !rels.iter().any(|r| r.contains(".git")),
            "no nested .git entries should appear: {rels:?}"
        );
    }

    // -----------------------------------------------------------------------
    // diff tests
    // -----------------------------------------------------------------------

    fn file_entry(path: &str, size: u64) -> Entry {
        Entry::File {
            path: path.to_string(),
            mode: 0o644,
            size,
            chunks: vec![ChunkRef {
                digest: Digest::of_bytes(path.as_bytes()),
                size,
            }],
        }
    }

    fn symlink_entry(path: &str, target: &str) -> Entry {
        Entry::Symlink {
            path: path.to_string(),
            target: target.to_string(),
        }
    }

    fn dir_entry(path: &str) -> Entry {
        Entry::Dir {
            path: path.to_string(),
        }
    }

    #[test]
    fn diff_added() {
        let old = make_manifest(vec![file_entry("a.txt", 1)]);
        let new = make_manifest(vec![file_entry("a.txt", 1), file_entry("b.txt", 2)]);
        let d = diff(&old, &new);
        assert_eq!(d.added, vec!["b.txt"]);
        assert!(d.removed.is_empty());
        assert!(d.changed.is_empty());
    }

    #[test]
    fn diff_removed() {
        let old = make_manifest(vec![file_entry("a.txt", 1), file_entry("b.txt", 2)]);
        let new = make_manifest(vec![file_entry("a.txt", 1)]);
        let d = diff(&old, &new);
        assert!(d.added.is_empty());
        assert_eq!(d.removed, vec!["b.txt"]);
        assert!(d.changed.is_empty());
    }

    #[test]
    fn diff_changed_size() {
        let old = make_manifest(vec![file_entry("a.txt", 1)]);
        let new = make_manifest(vec![file_entry("a.txt", 99)]);
        let d = diff(&old, &new);
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
        assert_eq!(d.changed, vec!["a.txt"]);
    }

    #[test]
    fn diff_changed_chunks() {
        let old = make_manifest(vec![Entry::File {
            path: "a.txt".to_string(),
            mode: 0o644,
            size: 5,
            chunks: vec![ChunkRef {
                digest: Digest::of_bytes(b"old"),
                size: 5,
            }],
        }]);
        let new = make_manifest(vec![Entry::File {
            path: "a.txt".to_string(),
            mode: 0o644,
            size: 5,
            chunks: vec![ChunkRef {
                digest: Digest::of_bytes(b"new"),
                size: 5,
            }],
        }]);
        let d = diff(&old, &new);
        assert_eq!(d.changed, vec!["a.txt"]);
    }

    // FIX 2 — a MODE-ONLY change (identical content/size/chunks, different mode)
    // must be reported as `changed`: the mode is in the manifest, so a re-snapshot
    // would produce a different manifest and `status` must not claim CLEAN.
    #[test]
    fn diff_changed_mode_only() {
        let chunks = vec![ChunkRef {
            digest: Digest::of_bytes(b"same"),
            size: 5,
        }];
        let old = make_manifest(vec![Entry::File {
            path: "exec.sh".to_string(),
            mode: 0o644,
            size: 5,
            chunks: chunks.clone(),
        }]);
        // Only the mode differs (chmod +x); content/size/chunks identical.
        let new = make_manifest(vec![Entry::File {
            path: "exec.sh".to_string(),
            mode: 0o755,
            size: 5,
            chunks,
        }]);
        let d = diff(&old, &new);
        assert_eq!(
            d.changed,
            vec!["exec.sh"],
            "a mode-only change must be reported as changed"
        );
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
    }

    // FIX 2 (unix) — end to end through `walk`: a tracked file whose MODE changed
    // (content byte-identical) yields a manifest that diffs as `changed` against
    // the pre-chmod manifest. This proves status & snapshot agree on mode masking.
    #[cfg(unix)]
    #[test]
    fn walk_mode_change_is_reported_as_changed() {
        use std::os::unix::fs::PermissionsExt;

        // Build a manifest from a walk of `root` (File entries only; no chunking —
        // diff compares size+chunks+mode and content is unchanged so chunks stay
        // identical, but to keep this manifest-only we synthesize chunks from rel).
        fn manifest_from_walk(root: &Path) -> Manifest {
            let entries: Vec<Entry> = walk(root)
                .unwrap()
                .into_iter()
                .filter_map(|e| match e {
                    WalkedEntry::File(f) => Some(Entry::File {
                        path: f.rel.clone(),
                        mode: f.mode,
                        size: f.size,
                        // Content is identical across the chmod, so a content-derived
                        // digest is identical too — isolating MODE as the only delta.
                        chunks: vec![ChunkRef {
                            digest: Digest::of_bytes(f.rel.as_bytes()),
                            size: f.size,
                        }],
                    }),
                    _ => None,
                })
                .collect();
            make_manifest(entries)
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let path = root.join("exec.sh");
        fs::write(&path, b"#!/bin/sh\n").unwrap();

        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).unwrap();
        let before = manifest_from_walk(root);

        // chmod +x — content byte-identical, only the mode changes.
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        let after = manifest_from_walk(root);

        let d = diff(&before, &after);
        assert_eq!(
            d.changed,
            vec!["exec.sh"],
            "a tracked file whose mode changed (content identical) must be reported as changed"
        );
    }

    #[test]
    fn diff_symlink_target_changed() {
        let old = make_manifest(vec![symlink_entry("link", "old_target")]);
        let new = make_manifest(vec![symlink_entry("link", "new_target")]);
        let d = diff(&old, &new);
        assert_eq!(d.changed, vec!["link"]);
    }

    #[test]
    fn diff_kind_change() {
        // Same path, different entry kind = changed.
        let old = make_manifest(vec![file_entry("thing", 10)]);
        let new = make_manifest(vec![symlink_entry("thing", "target")]);
        let d = diff(&old, &new);
        assert_eq!(d.changed, vec!["thing"]);
    }

    #[test]
    fn diff_no_change() {
        let entries = vec![
            file_entry("a.txt", 5),
            symlink_entry("link", "target"),
            dir_entry("emptydir"),
        ];
        let old = make_manifest(entries.clone());
        let new = make_manifest(entries);
        let d = diff(&old, &new);
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
        assert!(d.changed.is_empty());
    }

    #[test]
    fn diff_lists_sorted() {
        let old = make_manifest(vec![file_entry("c.txt", 1), file_entry("a.txt", 1)]);
        let new = make_manifest(vec![file_entry("b.txt", 2), file_entry("d.txt", 2)]);
        let d = diff(&old, &new);
        assert_eq!(d.added, vec!["b.txt", "d.txt"]);
        assert_eq!(d.removed, vec!["a.txt", "c.txt"]);
    }
}
