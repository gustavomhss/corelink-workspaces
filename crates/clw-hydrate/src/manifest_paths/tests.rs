use super::*;

fn link(path: &str, target: &str) -> Entry {
    Entry::Symlink {
        path: path.into(),
        target: target.into(),
    }
}
fn dir(path: &str) -> Entry {
    Entry::Dir { path: path.into() }
}
fn file(path: &str) -> Entry {
    Entry::File {
        path: path.into(),
        mode: 0o600,
        size: 0,
        chunks: vec![],
    }
}
fn manifest(mut entries: Vec<Entry>) -> Manifest {
    entries.sort_by(|a, b| a.path().cmp(b.path()));
    Manifest {
        version: clw_types::MANIFEST_VERSION,
        total_size: 0,
        entries,
    }
}

#[test]
fn leaf_ancestors_are_rejected_in_both_insertion_orders() {
    for leaf in [file("node"), link("node", "target")] {
        for reverse in [false, true] {
            let mut m = manifest(vec![leaf.clone(), file("node/item"), dir("target")]);
            if reverse {
                m.entries.reverse();
            }
            assert!(validate(&m).unwrap_err().to_string().contains("ancestor"));
        }
    }
}

#[test]
fn similar_prefixes_and_explicit_directories_are_not_leaf_ancestors() {
    validate(&manifest(vec![file("a"), dir("ab"), file("ab/c")])).unwrap();
    validate(&manifest(vec![dir("a"), file("a/c")])).unwrap();
}

#[test]
fn links_resolve_before_following_parent_components() {
    // Only the new pure validator runs; no symlinks or filesystem effects.
    let m = manifest(vec![
        dir("dir"),
        link("dir/alias", ".."),
        link("use", "dir/alias/.."),
    ]);
    assert!(validate(&m)
        .unwrap_err()
        .to_string()
        .contains("leaves the destination"));
    let safe = manifest(vec![
        dir("dir/sub"),
        file("dir/value"),
        link("dir/alias", "sub"),
        link("use", "dir/alias/../value"),
    ]);
    validate(&safe).unwrap();
}

#[test]
fn forward_links_and_safe_dangling_suffixes_remain_valid() {
    validate(&manifest(vec![
        link("a", "b"),
        link("b", "z/value"),
        file("z/value"),
    ]))
    .unwrap();
    validate(&manifest(vec![link("a", "missing/child")])).unwrap();
    validate(&manifest(vec![dir("d"), link("d/up", "..")])).unwrap();
    validate(&manifest(vec![
        file("Ä/sub/value"),
        link("Ä/sub/up", "../../ä/sub"),
    ]))
    .unwrap();
}

#[test]
fn uncertain_parent_traversal_and_file_traversal_are_refused() {
    assert!(validate(&manifest(vec![link("alias", "missing/../value")])).is_err());
    assert!(validate(&manifest(vec![file("value"), link("alias", "value/child")])).is_err());
}

#[test]
fn cycles_and_long_expansions_have_a_finite_refusal() {
    assert!(validate(&manifest(vec![link("a", "b"), link("b", "a")]))
        .unwrap_err()
        .to_string()
        .contains("cycle"));
    let mut entries = vec![file("value")];
    for n in (0..MAX_LINK_EXPANSIONS + 2).rev() {
        let target = if n == MAX_LINK_EXPANSIONS + 1 {
            "value".into()
        } else {
            format!("link{:03}", n + 1)
        };
        entries.push(link(&format!("link{n:03}"), &target));
    }
    assert!(validate(&manifest(entries)).is_err());
}

#[test]
fn queue_and_operation_budgets_are_real_limits() {
    let nodes = vec![Node::directory(None)];
    let mut budget = 1;
    assert!(resolve(&nodes, 0, "alias", "./.", &mut budget).is_err());

    let mut pending = VecDeque::new();
    let exact = vec!["."; MAX_PENDING_COMPONENTS].join("/");
    prepend(&mut pending, &exact, "alias").unwrap();
    assert_eq!(pending.len(), MAX_PENDING_COMPONENTS);

    let mut pending = VecDeque::new();
    let oversized = vec!["."; MAX_PENDING_COMPONENTS + 1].join("/");
    assert!(prepend(&mut pending, &oversized, "alias").is_err());
    assert!(pending.is_empty());
}

#[test]
fn exact_link_expansion_limit_remains_accepted() {
    let mut entries = vec![file("value")];
    for n in (0..=MAX_LINK_EXPANSIONS).rev() {
        let target = if n == MAX_LINK_EXPANSIONS {
            "value".into()
        } else {
            format!("link{:03}", n + 1)
        };
        entries.push(link(&format!("link{n:03}"), &target));
    }
    validate(&manifest(entries)).unwrap();
}

#[test]
fn every_link_parent_is_prepared_as_a_real_directory() {
    let root = tempfile::tempdir().unwrap();
    let m = manifest(vec![link("a/b/alias", "../../missing")]);
    validate(&m).unwrap();
    prepare_directories(root.path(), &m).unwrap();
    assert!(std::fs::symlink_metadata(root.path().join("a/b"))
        .unwrap()
        .is_dir());
    check_link_parent(root.path(), "a/b/alias").unwrap();
}

#[test]
fn explicit_empty_directory_is_prepared_before_materialization() {
    let root = tempfile::tempdir().unwrap();
    prepare_directories(root.path(), &manifest(vec![dir("empty")])).unwrap();
    assert!(std::fs::symlink_metadata(root.path().join("empty"))
        .unwrap()
        .file_type()
        .is_dir());
}

#[test]
fn existing_contained_symlink_is_never_used_as_a_parent() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("real")).unwrap();
    std::os::unix::fs::symlink("real", root.path().join("alias")).unwrap();
    assert!(check_link_parent(root.path(), "alias/child").is_err());
    assert!(prepare_directories(root.path(), &manifest(vec![file("alias/child/value")])).is_err());
    assert!(!root.path().join("real/child").exists());
}

#[test]
fn directory_creation_surfaces_permission_error_without_reclassification() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = directories(root.path(), "child", true);
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        result,
        Err(ClwError::Io(ref error)) if error.kind() == std::io::ErrorKind::PermissionDenied
    ));
}

#[test]
fn invalid_paths_duplicates_and_targets_do_not_enter_the_tree() {
    assert!(validate(&manifest(vec![file("../invalid")])).is_err());
    assert!(validate(&manifest(vec![file("same"), file("same")])).is_err());
    assert!(validate(&manifest(vec![link("alias", "/invalid")])).is_err());
}

#[test]
fn reservation_permission_failure_is_io_not_collision() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = LinkSlots::reserve(root.path(), &manifest(vec![link("alias", ".")]));
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        result,
        Err(ClwError::Io(ref error)) if error.kind() == std::io::ErrorKind::PermissionDenied
    ));
}

#[test]
fn reservation_collision_cleans_only_owned_placeholders() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("z"), b"preserve").unwrap();
    let m = manifest(vec![link("a", "."), link("z", ".")]);
    let error = match LinkSlots::reserve(root.path(), &m) {
        Ok(_) => panic!("reservation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ClwError::InvalidManifest(ref message) if message.contains("symlink name collides")
    ));
    assert!(!root.path().join("a").exists());
    assert_eq!(std::fs::read(root.path().join("z")).unwrap(), b"preserve");
}

#[test]
fn filesystem_aliases_are_decided_before_any_link_is_published() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Probe"), b"probe").unwrap();
    let insensitive = root.path().join("pROBE").exists();
    std::fs::remove_file(root.path().join("Probe")).unwrap();
    let m = manifest(vec![link("Alias", "."), link("alias", ".")]);
    let result = LinkSlots::reserve(root.path(), &m);
    if insensitive {
        assert!(result.is_err());
    } else {
        assert!(result.is_ok());
    }
    drop(result);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    println!("link_name_admission case_insensitive={insensitive}");
}

#[test]
fn published_link_is_not_removed_with_pending_placeholders() {
    let root = tempfile::tempdir().unwrap();
    let m = manifest(vec![link("a", "."), link("b", ".")]);
    let mut slots = LinkSlots::reserve(root.path(), &m).unwrap();
    assert!(slots.publish(root.path(), "b", ".").is_err());
    slots.publish(root.path(), "a", ".").unwrap();
    drop(slots);
    assert_eq!(
        std::fs::read_link(root.path().join("a")).unwrap(),
        Path::new(".")
    );
    assert!(std::fs::symlink_metadata(root.path().join("b")).is_err());
}
