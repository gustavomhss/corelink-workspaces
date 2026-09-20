//! Destination name representability, without copying workspace payloads.
//! Literal names remain unchanged. The filesystem, not case folding, decides
//! whether two names collide. The probe contains no manifest-provided symlinks.
use clw_types::{ClwError, Entry, Manifest, Result};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{collections::btree_map::Entry as MapEntry, collections::BTreeMap, io, path::Path};

fn collision(path: &str) -> ClwError {
    ClwError::InvalidManifest(format!(
        "destination namespace collision at {path:?}; the destination cannot represent distinct snapshot paths"
    ))
}

/// Borrow source spellings rather than allocate a second set of path strings.
/// Prefixes count towards the same finite node budget as explicit entries.
fn namespace(manifest: &Manifest, limit: usize) -> Result<BTreeMap<&str, bool>> {
    let mut nodes = BTreeMap::new();
    for entry in &manifest.entries {
        let path = entry.path();
        clw_types::validate_rel_path(path)?;
        for end in path
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(path.len()))
        {
            let name = &path[..end];
            let directory = end != path.len() || matches!(entry, Entry::Dir { .. });
            let full = nodes.len() >= limit;
            match nodes.entry(name) {
                MapEntry::Occupied(old) if directory && *old.get() => {}
                MapEntry::Occupied(_) => return Err(collision(name)),
                MapEntry::Vacant(_) if full => {
                    return Err(ClwError::InvalidManifest(format!(
                        "destination namespace exceeds {limit} nodes including implicit directories; refusing to hydrate"
                    )));
                }
                MapEntry::Vacant(slot) => {
                    slot.insert(directory);
                }
            }
        }
    }
    Ok(nodes)
}

/// Called on the blocking pool before payload fetches or final-name creation.
/// The empty destination may remain after an error; only the owned probe is
/// removed. No probe file contains user data or points outside the private tree.
pub(super) fn validate(dest: &Path, manifest: &Manifest) -> Result<()> {
    let nodes = namespace(manifest, super::MAX_MANIFEST_ENTRIES)?;
    if nodes.is_empty() {
        return Ok(());
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix(".clw-names-");
    #[cfg(unix)]
    builder.permissions(std::fs::Permissions::from_mode(0o700));
    let probe = builder.tempdir_in(dest)?;
    let mut created_count = 0;
    let checked = (|| -> Result<()> {
        // Lexical ordering places every literal parent before its children.
        // Only identical literal directories were deduplicated above. An OS
        // AlreadyExists here therefore denotes an alias, never a shared parent.
        for (&name, &directory) in &nodes {
            let path = probe.path().join(name);
            let created = if directory {
                std::fs::create_dir(&path)
            } else {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                options.open(&path).map(drop)
            };
            if let Err(error) = created {
                return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                    collision(name)
                } else {
                    ClwError::Io(io::Error::new(
                        error.kind(),
                        format!("cannot represent destination path {name:?}: {error}"),
                    ))
                });
            }
            created_count += 1;
        }
        Ok(())
    })();
    // Cleanup failure prevents acceptance; an earlier collision remains the
    // primary error. Never recursively remove the destination itself.
    #[cfg(test)]
    let diagnostic_path = probe.path().to_path_buf();
    // Use only the successful prefix as the ownership journal. Disarm the
    // recursive fallback: a failed alias must not redirect cleanup enumeration.
    // Unknown entries make final remove_dir fail instead of being deleted.
    let probe_root = probe.keep();
    let cleanup = cleanup_probe(&probe_root, &nodes, created_count);
    #[cfg(test)]
    if let Err(error) = &cleanup {
        eprintln!(
            "CLW303_PROBE_CLEANUP kind={:?} errno={:?} error={error}",
            error.kind(),
            error.raw_os_error()
        );
        if let Ok(entries) = std::fs::read_dir(&diagnostic_path) {
            for entry in entries.take(16) {
                match entry {
                    Ok(entry) => eprintln!(
                        "CLW303_PROBE_REMAINS name={:?} type={:?}",
                        entry.file_name(),
                        entry.file_type()
                    ),
                    Err(error) => eprintln!("CLW303_PROBE_ENUM_ERROR {error}"),
                }
            }
        }
    }
    match (checked, cleanup) {
        (Err(error), Err(cleanup)) => {
            tracing::warn!(error = %cleanup, "destination name probe cleanup failed");
            Err(error)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), cleanup) => cleanup.map_err(ClwError::Io),
    }
}

/// Remove successfully created names in reverse order, children before parents.
/// The ordered index is the journal; no second manifest-sized allocation is needed.
/// Keep the first error while still trying the other owned removals. Unknown
/// entries are never enumerated or recursively removed.
fn cleanup_probe(root: &Path, nodes: &BTreeMap<&str, bool>, created: usize) -> io::Result<()> {
    let mut first_error = None;
    for (&name, &directory) in nodes.iter().take(created).rev() {
        let path = root.join(name);
        let result = if directory {
            std::fs::remove_dir(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Err(error) = std::fs::remove_dir(root) {
        if error.kind() != io::ErrorKind::NotFound && first_error.is_none() {
            first_error = Some(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dirs(paths: &[&str]) -> Manifest {
        Manifest {
            version: 1,
            total_size: 0,
            entries: paths
                .iter()
                .map(|path| Entry::Dir {
                    path: (*path).into(),
                })
                .collect(),
        }
    }
    #[test]
    fn implicit_nodes_are_budgeted_and_shared_literal_parents_are_reused() {
        assert!(namespace(&dirs(&["one/two/three"]), 2).is_err());
        assert_eq!(
            namespace(&dirs(&["one", "one/two", "one/three"]), 3)
                .unwrap()
                .len(),
            3
        );
        assert!(namespace(&dirs(&["one", "two"]), 1).is_err());
        assert!(namespace(&dirs(&[]), 0).unwrap().is_empty());
    }
    #[test]
    fn non_directory_entries_cannot_be_literal_parents() {
        let mut manifest = dirs(&["leaf/child"]);
        manifest.entries.insert(
            0,
            Entry::Symlink {
                path: "leaf".into(),
                target: "target".into(),
            },
        );
        assert!(namespace(&manifest, 10).is_err());
    }

    #[test]
    fn exact_cleanup_removes_children_before_parents() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("probe");
        std::fs::create_dir_all(root.join("Upper/deep")).unwrap();
        std::fs::write(root.join("Upper/deep/file"), b"").unwrap();
        let nodes = BTreeMap::from([
            ("Upper", true),
            ("Upper/deep", true),
            ("Upper/deep/file", false),
        ]);
        cleanup_probe(&root, &nodes, nodes.len()).unwrap();
        assert!(!root.exists());
        assert!(parent.path().is_dir());
    }

    #[test]
    fn exact_cleanup_preserves_an_uncreated_suffix_and_reports_residue() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("probe");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a-owned"), b"").unwrap();
        std::fs::write(root.join("z-not-created"), b"synthetic outsider").unwrap();
        let nodes = BTreeMap::from([("a-owned", false), ("z-not-created", false)]);
        assert!(cleanup_probe(&root, &nodes, 1).is_err());
        assert!(!root.join("a-owned").exists());
        assert_eq!(
            std::fs::read(root.join("z-not-created")).unwrap(),
            b"synthetic outsider"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn exact_cleanup_tolerates_an_already_absent_owned_name() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("probe");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a-present"), b"").unwrap();
        let nodes = BTreeMap::from([("a-present", false), ("z-already-absent", false)]);
        cleanup_probe(&root, &nodes, nodes.len()).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn exact_cleanup_tolerates_an_already_absent_probe_root() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("already-absent-probe");
        let nodes = BTreeMap::new();
        cleanup_probe(&root, &nodes, 0).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn exact_cleanup_continues_after_an_owned_removal_failure() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("probe");
        std::fs::create_dir_all(root.join("z-changed-type")).unwrap();
        std::fs::write(root.join("z-changed-type/retain"), b"synthetic outsider").unwrap();
        std::fs::write(root.join("a-owned"), b"").unwrap();
        let nodes = BTreeMap::from([("a-owned", false), ("z-changed-type", false)]);
        assert!(cleanup_probe(&root, &nodes, 2).is_err());
        assert!(
            !root.join("a-owned").exists(),
            "cleanup stopped at the first error"
        );
        assert_eq!(
            std::fs::read(root.join("z-changed-type/retain")).unwrap(),
            b"synthetic outsider"
        );
    }
}
