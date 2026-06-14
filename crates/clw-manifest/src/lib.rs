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
    Symlink { rel: String, target: String },
    EmptyDir { rel: String },
}

impl WalkedEntry {
    fn rel(&self) -> &str {
        match self {
            WalkedEntry::File(f) => &f.rel,
            WalkedEntry::Symlink { rel, .. } => rel,
            WalkedEntry::EmptyDir { rel } => rel,
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
            let target = link_target
                .to_str()
                .ok_or_else(|| {
                    ClwError::InvalidManifest(format!(
                        "non-utf8 path: {}",
                        link_target.to_string_lossy()
                    ))
                })?
                .to_string();
            entries.push(WalkedEntry::Symlink { rel, target });
        } else if file_type.is_dir() {
            // Record as a candidate empty dir; we'll filter after collecting all entries.
            candidate_dirs.push((rel, abs));
        } else if file_type.is_file() {
            let meta = std::fs::metadata(&abs)?;
            // On unix, read the real permission bits. On Windows there is no
            // unix mode concept, so we record a constant default of 0o644 for
            // every regular file (symlink DETECTION below stays cross-platform).
            #[cfg(unix)]
            let mode = meta.permissions().mode() & 0o7777;
            #[cfg(windows)]
            let mode = 0o644;
            let size = meta.len();
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
                size: old_size,
                chunks: old_chunks,
                ..
            },
            Entry::File {
                size: new_size,
                chunks: new_chunks,
                ..
            },
        ) => old_size != new_size || old_chunks != new_chunks,
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
    use std::os::unix::fs::PermissionsExt;
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

    #[test]
    fn walk_mode_captured() {
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
