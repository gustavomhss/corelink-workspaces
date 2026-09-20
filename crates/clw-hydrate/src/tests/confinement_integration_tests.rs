use super::*;

#[tokio::test]
async fn confinement_preflight_refuses_file_ancestor_before_destination_creation() {
    // A harmless inconsistent tree, never an out-of-root/escape reproduction.
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        total_size: 0,
        entries: vec![
            Entry::File {
                path: "node".into(),
                mode: 0o600,
                size: 0,
                chunks: vec![],
            },
            Entry::File {
                path: "node/item".into(),
                mode: 0o600,
                size: 0,
                chunks: vec![],
            },
        ],
    };
    for direct in [false, true] {
        let (store, _cache_root, cache) = setup_store(&manifest, &[]);
        let parent = TempDir::new().unwrap();
        let dest = parent.path().join("out");
        let opts = if direct {
            HydrateOptions::from_manifest_digest(manifest.digest().unwrap())
        } else {
            HydrateOptions::new("test-ws")
        };
        assert!(hydrate(&dest, &store, &cache, &opts).await.is_err());
        assert!(
            !dest.exists(),
            "invalid structure must fail before destination creation"
        );
    }
}

#[tokio::test]
async fn confinement_exact_composed_link_specimen_refuses_before_destination_creation() {
    // Exact #296 manifest shape. The canonical parser accepts these bytes; the
    // materializer-specific whole-tree admission must reject the composition
    // before creating `dest`, so this regression never performs the historical
    // out-of-root filesystem effect.
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        total_size: 0,
        entries: vec![
            Entry::Symlink {
                path: "a".into(),
                target: ".".into(),
            },
            Entry::Symlink {
                path: "b".into(),
                target: "a/..".into(),
            },
            Entry::Symlink {
                path: "b/outside-marker".into(),
                target: ".".into(),
            },
        ],
    };
    let encoded = manifest.canonical_bytes().unwrap();
    let parsed =
        Manifest::from_bytes(&encoded).expect("wire-format parser still accepts the specimen");
    assert_eq!(parsed.entries.len(), 3);

    for direct in [false, true] {
        let (store, _cache_root, cache) = setup_store(&parsed, &[]);
        let parent = TempDir::new().unwrap();
        let dest = parent.path().join("dest");
        let outside = parent.path().join("outside-marker");
        let opts = if direct {
            HydrateOptions::from_manifest_digest(parsed.digest().unwrap())
        } else {
            HydrateOptions::new("test-ws")
        };
        let error = hydrate(&dest, &store, &cache, &opts)
            .await
            .expect_err("composed symlink graph must be refused");
        assert!(matches!(error, ClwError::InvalidManifest(_)), "{error:?}");
        assert!(
            !dest.exists(),
            "the exact specimen must fail before destination creation"
        );
        assert!(
            !outside.exists(),
            "rejected specimen must not create the historical external marker"
        );
    }
}

#[tokio::test]
async fn confinement_safe_forward_links_preserve_bytes_and_target_spelling() {
    let data = b"ordinary workspace content";
    let (digest, bytes) = make_chunk(data);
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        total_size: data.len() as u64,
        entries: vec![
            Entry::Symlink {
                path: "a".into(),
                target: "b".into(),
            },
            Entry::Symlink {
                path: "b".into(),
                target: "dir/value".into(),
            },
            Entry::Symlink {
                path: "bin/tool".into(),
                target: "../dir/value".into(),
            },
            Entry::File {
                path: "dir/value".into(),
                mode: 0o600,
                size: data.len() as u64,
                chunks: vec![ChunkRef {
                    digest,
                    size: data.len() as u64,
                }],
            },
        ],
    };
    let (store, _cache_root, cache) = setup_store(&manifest, &[(digest, bytes)]);
    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("out");
    let report = hydrate(&dest, &store, &cache, &HydrateOptions::new("test-ws"))
        .await
        .unwrap();
    assert_eq!(report.files, 1);
    for name in ["a", "b", "bin/tool", "dir/value"] {
        assert_eq!(std::fs::read(dest.join(name)).unwrap(), data);
        assert!(std::fs::canonicalize(dest.join(name))
            .unwrap()
            .starts_with(std::fs::canonicalize(&dest).unwrap()));
    }
    assert_eq!(
        std::fs::read_link(dest.join("bin/tool")).unwrap(),
        PathBuf::from("../dir/value")
    );
}
