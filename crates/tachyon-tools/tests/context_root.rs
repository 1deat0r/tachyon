//! M11 slice 5 (pin/policy single-source): the gateway pins ONE durable
//! canonical root and then draws every boundary from that exact value.
//! `ToolsContext::new_from_canonical` must perform NO second filesystem
//! resolution — the pinned `PathBuf` value itself becomes the policy,
//! evidence and mutation root, so no await window between the pin and
//! the context construction can make them diverge. The contrast case
//! proves this test would catch a reintroduced re-canonicalization.
#![cfg(unix)]

use std::path::PathBuf;

use tachyon_policy::{DefaultPosture, Policy};
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};

#[test]
fn canonical_constructor_passes_the_pinned_root_through_unchanged() {
    let dir = std::env::temp_dir().join(format!(
        "tachyon-pin-root-{}-{}",
        std::process::id(),
        tachyon_types::TaskId::generate()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let real = dir.join("real");
    let link = dir.join("link");
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let policy = Policy::new(DefaultPosture::Ask);
    let pinned: PathBuf = link.clone();

    // The single-source constructor: the caller already canonicalized
    // (gateway prepare step 3), so the value must arrive byte-identical.
    let context = ToolsContext::new_from_canonical(
        pinned.clone(),
        policy.clone(),
        ArtifactSpool::new(dir.join("spool-a")),
    );
    assert_eq!(
        context.workspace_root, pinned,
        "no second resolution: the pinned root must be the policy/evidence \
         mutation root byte-for-byte"
    );

    // Contrast: the best-effort constructor DOES resolve the symlink —
    // this is exactly the divergence the pin path must not have.
    let resolving = ToolsContext::new(
        pinned.clone(),
        policy,
        ArtifactSpool::new(dir.join("spool-b")),
    );
    assert_ne!(
        resolving.workspace_root, pinned,
        "the resolving constructor must resolve, or this test proves nothing"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
