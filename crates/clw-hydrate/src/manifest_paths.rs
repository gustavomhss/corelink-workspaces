//! Materialization-only path validation. The manifest wire format is unchanged.
//!
//! Build a directory tree, reject leaf ancestors, and resolve link components in
//! expansion order before any destination write. This is deliberately not a
//! lexical collapse of each target. The directory skeleton is then created before
//! links, and link parents are checked without following links. These checks own
//! manifest-created paths; they are not exclusion against hostile local writers.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use clw_types::{ClwError, Entry, Manifest, Result};

const MAX_LINK_EXPANSIONS: usize = 40;
const MAX_PENDING_COMPONENTS: usize = 4096;
const MAX_RESOLUTION_STEPS: usize = 4_000_000;

#[derive(Clone, Copy)]
enum Kind<'a> {
    Directory,
    File,
    Link(&'a str),
}

struct Node<'a> {
    parent: Option<usize>,
    children: BTreeMap<&'a str, usize>,
    kind: Kind<'a>,
    declared: bool,
}

impl Node<'_> {
    fn directory(parent: Option<usize>) -> Self {
        Self {
            parent,
            children: BTreeMap::new(),
            kind: Kind::Directory,
            declared: false,
        }
    }
}

fn invalid(path: &str, reason: &str) -> ClwError {
    ClwError::InvalidManifest(format!("invalid materialization path {path:?}: {reason}"))
}

/// Borrowed trie: bookkeeping is linear in encoded path components, not in all
/// possible prefix strings. IDs refer only to nodes already inserted in `nodes`.
pub(super) fn validate(manifest: &Manifest) -> Result<()> {
    let mut nodes = vec![Node::directory(None)];
    let mut links = Vec::new();
    for entry in &manifest.entries {
        let path = entry.path();
        clw_types::validate_rel_path(path)?;
        let mut current = 0;
        for component in path.split('/') {
            if !matches!(nodes[current].kind, Kind::Directory) {
                return Err(invalid(path, "a file or symlink cannot be an ancestor"));
            }
            let child = match nodes[current].children.get(component).copied() {
                Some(child) => child,
                None => {
                    let child = nodes.len();
                    nodes.push(Node::directory(Some(current)));
                    nodes[current].children.insert(component, child);
                    child
                }
            };
            current = child;
        }
        if nodes[current].declared {
            return Err(invalid(path, "duplicate entry"));
        }
        if !matches!(entry, Entry::Dir { .. }) && !nodes[current].children.is_empty() {
            return Err(invalid(path, "a file or symlink cannot be an ancestor"));
        }
        nodes[current].declared = true;
        nodes[current].kind = match entry {
            Entry::Dir { .. } => Kind::Directory,
            Entry::File { .. } => Kind::File,
            Entry::Symlink { target, .. } => {
                clw_types::validate_symlink_target_contained(path, target)?;
                links.push((current, path, target.as_str()));
                Kind::Link(target)
            }
        };
    }
    let mut budget = MAX_RESOLUTION_STEPS;
    for (id, path, target) in links {
        let parent = nodes[id]
            .parent
            .ok_or_else(|| invalid(path, "link has no parent"))?;
        resolve(&nodes, parent, path, target, &mut budget)?;
    }
    Ok(())
}

fn prepend<'a>(pending: &mut VecDeque<&'a str>, target: &'a str, path: &str) -> Result<()> {
    let count = target.split('/').count();
    if count > MAX_PENDING_COMPONENTS.saturating_sub(pending.len()) {
        return Err(invalid(
            path,
            "symlink resolution component budget exceeded",
        ));
    }
    for component in target.rsplit('/') {
        pending.push_front(component);
    }
    Ok(())
}

fn resolve<'a>(
    nodes: &[Node<'a>],
    mut current: usize,
    path: &str,
    target: &'a str,
    budget: &mut usize,
) -> Result<()> {
    let mut pending = VecDeque::new();
    prepend(&mut pending, target, path)?;
    let mut expansions = 0;
    while let Some(component) = pending.pop_front() {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| invalid(path, "symlink resolution work budget exceeded"))?;
        match component {
            "." => continue,
            ".." => {
                current = nodes[current]
                    .parent
                    .ok_or_else(|| invalid(path, "symlink resolution leaves the destination"))?;
                continue;
            }
            _ => {}
        }
        let Some(child) = nodes[current].children.get(component).copied() else {
            // A dangling normal suffix is legal on Unix. A parent step after
            // an unknown component is not provably confined (the OS may expand
            // a differently spelled alias there); do not guess its depth.
            return if pending.contains(&"..") {
                Err(invalid(
                    path,
                    "parent traversal after an unresolved target component",
                ))
            } else {
                Ok(())
            };
        };
        match nodes[child].kind {
            Kind::Directory => current = child,
            Kind::File if pending.is_empty() => return Ok(()),
            Kind::File => return Err(invalid(path, "symlink target traverses a file")),
            Kind::Link(next) => {
                expansions += 1;
                if expansions > MAX_LINK_EXPANSIONS {
                    return Err(invalid(path, "symlink cycle or expansion limit exceeded"));
                }
                // `current` is the link's containing directory. Expand the
                // target THERE, before processing the original suffix.
                prepend(&mut pending, next, path)?;
            }
        }
    }
    Ok(())
}

/// Create all declared and implicit directories before files/links. In
/// particular, parents needed only by links must exist before the first link;
/// a case-folded link name then cannot replace an implicit directory.
pub(super) fn prepare_directories(root: &Path, manifest: &Manifest) -> Result<()> {
    for entry in &manifest.entries {
        let path = entry.path();
        let directory = match entry {
            Entry::Dir { .. } => path,
            _ => path.rsplit_once('/').map_or("", |(parent, _)| parent),
        };
        directories(root, directory, true)?;
    }
    Ok(())
}

/// Recheck real parent objects immediately before link publication. No new
/// directories are created once the link pass has begun.
pub(super) fn check_link_parent(root: &Path, path: &str) -> Result<()> {
    let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
    directories(root, parent, false)
}

fn directories(root: &Path, relative: &str, create: bool) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.split('/').filter(|c| !c.is_empty()) {
        current.push(component);
        if create {
            match std::fs::create_dir(&current) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let metadata = std::fs::symlink_metadata(&current)?;
        if !metadata.file_type().is_dir() {
            return Err(invalid(relative, "parent is not a plain directory"));
        }
    }
    Ok(())
}

/// Claim every link name exclusively before publishing ANY link. This lets the
/// filesystem reject case/normalization aliases that a literal manifest trie
/// cannot know. Otherwise a later colliding link could invalidate the graph used
/// to validate earlier links. Only empty, owner-only placeholders are owned here.
pub(super) struct LinkSlots {
    pending: VecDeque<std::path::PathBuf>,
}

impl LinkSlots {
    pub(super) fn reserve(root: &Path, manifest: &Manifest) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut slots = Self {
            pending: VecDeque::new(),
        };
        for entry in &manifest.entries {
            if let Entry::Symlink { path, .. } = entry {
                check_link_parent(root, path)?;
                let absolute = root.join(path);
                let file = match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&absolute)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        return Err(invalid(
                            path,
                            "symlink name collides with another destination entry",
                        ));
                    }
                    Err(error) => return Err(error.into()),
                };
                // Arm cleanup only for names this operation actually created.
                slots.pending.push_back(absolute);
                drop(file);
            }
        }
        Ok(slots)
    }

    pub(super) fn publish(&mut self, root: &Path, path: &str, target: &str) -> Result<()> {
        let absolute = root.join(path);
        if self.pending.front() != Some(&absolute) {
            return Err(invalid(path, "symlink publication does not own this name"));
        }
        check_link_parent(root, path)?;
        std::fs::remove_file(&absolute)?;
        // A subsequent creation failure must not remove an unowned occupant.
        self.pending.pop_front();
        std::os::unix::fs::symlink(target, absolute)?;
        Ok(())
    }
}

impl Drop for LinkSlots {
    fn drop(&mut self) {
        for path in &self.pending {
            // Only unpublished placeholders remain; published links are removed
            // from this collection before creation and are never deleted here.
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests;
