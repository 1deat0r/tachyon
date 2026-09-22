//! M10: authorized preparation — exact policy before any effect.
//!
//! `prepare_authorized` binds a caller-reserved batch identity to exact target
//! and derived temp permissions, literal paths, unoccupied staging paths and
//! exact preimages. Every refusal below must leave the workspace, the artifact
//! spool and the journal untouched.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use tachyon_mutation::{BatchJournal, MutationEngine, MutationError, PatchSpec, blake3_hex};
use tachyon_policy::{DefaultPosture, Policy};
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::MutationBatchId;

const BEFORE: &[u8] = b"before";
const AFTER: &[u8] = b"after";

struct Fixture {
    root: PathBuf,
    ws: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tachyon-authorized-{}",
            MutationBatchId::generate()
        ));
        std::fs::create_dir_all(root.join("workspace").join("sub")).expect("workspace");
        // Canonicalize once so canonical engine paths and policy scopes
        // built from display strings agree (macOS /var → /private/var).
        let root = std::fs::canonicalize(&root).expect("canonical root");
        let ws = root.join("workspace");
        let state = root.join("state");
        std::fs::write(ws.join("a.rs"), BEFORE).expect("source");
        std::fs::write(ws.join("sub/b.rs"), BEFORE).expect("source");
        Self { root, ws, state }
    }

    fn engine(&self) -> MutationEngine {
        MutationEngine::open(&self.ws, &self.state).expect("engine")
    }

    fn context(&self) -> ToolsContext {
        let mut policy = Policy::trusted_workspace();
        policy.allow("mutation.patch", "workspace/**");
        policy.allow("fs.delete", "workspace/**");
        for capability in ["fs.read", "fs.metadata"] {
            policy.allow(
                capability,
                &format!("external:{}/artifacts/**", self.state.display()),
            );
        }
        ToolsContext::new(
            self.ws.clone(),
            policy,
            ArtifactSpool::new(self.root.join("tool-artifacts")),
        )
    }

    fn write(&self, path: &str, content: &[u8]) {
        let path = self.ws.join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parents");
        std::fs::write(path, content).expect("fixture write");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn specs() -> [PatchSpec; 2] {
    ["a.rs", "sub/b.rs"].map(|path| PatchSpec {
        path: path.to_owned(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: AFTER.to_vec(),
    })
}

fn spec(path: &str) -> PatchSpec {
    PatchSpec {
        path: path.to_owned(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: AFTER.to_vec(),
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).expect("snapshot dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let key = path.strip_prefix(root).expect("relative").to_path_buf();
            let kind = entry.file_type().expect("type");
            let value = if kind.is_symlink() {
                std::fs::read_link(&path)
                    .expect("link")
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else if kind.is_dir() {
                dirs.push(path.clone());
                b"<directory>".to_vec()
            } else {
                std::fs::read(&path).expect("read fixture")
            };
            result.insert(key, value);
        }
    }
    result
}

fn journaled_batches(
    fixture: &Fixture,
) -> BTreeMap<MutationBatchId, tachyon_mutation::ReplayedBatch> {
    BatchJournal::open(&fixture.state)
        .expect("journal")
        .replay()
        .expect("replay")
}

#[test]
fn authorized_prepare_preflights_every_exact_permission_before_effects() {
    for (capability, on_temp) in [
        ("fs.metadata", false),
        ("fs.read", false),
        ("mutation.patch", false),
        ("fs.write", false),
        ("fs.metadata", true),
        ("fs.write", true),
        ("fs.delete", true),
    ] {
        let fixture = Fixture::new();
        let engine = fixture.engine();
        let id = MutationBatchId::generate();
        let mut context = fixture.context();
        let scope = if on_temp {
            format!("workspace/sub/.b.rs.tachyon-tmp-{id}")
        } else {
            "workspace/sub/b.rs".to_owned()
        };
        // The exact denial must win even behind trusted-workspace broad grants.
        context.policy.deny(capability, &scope);
        let before = snapshot(&fixture.root);
        let error = engine
            .prepare_authorized(&context, id, &specs())
            .expect_err("exact permission denied");
        assert!(error.to_string().contains(capability), "{error}");
        assert_eq!(snapshot(&fixture.root), before, "{capability}/{scope}");
        assert!(
            journaled_batches(&fixture).is_empty(),
            "{capability}/{scope}: no plan is journaled"
        );

        let prepared = engine
            .prepare_authorized(&fixture.context(), id, &specs())
            .expect("authorized preparation");
        assert_eq!(prepared.id, id, "caller-reserved batch identity");
        for (file, spec) in prepared.files.iter().zip(specs()) {
            assert_eq!(file.path, spec.path);
            assert_eq!(file.pre_hash, spec.base_hash);
            assert_eq!(file.post_hash, blake3_hex(&spec.new_content));
            let target = fixture.ws.join(&file.path);
            assert_eq!(std::fs::read(&target).expect("source"), BEFORE);
            assert_eq!(
                std::fs::read(target.with_file_name(&file.temp_name)).expect("temp"),
                spec.new_content
            );
        }
    }
}

#[test]
fn unresolved_ask_cannot_self_approve_a_preparation() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let mut context = fixture.context();
    // Only a.rs is granted `mutation.patch`; b.rs falls back to the Ask
    // default, which no engine-side state may approve.
    let mut policy = Policy::new(DefaultPosture::Ask);
    for capability in ["fs.metadata", "fs.read", "fs.write", "fs.delete"] {
        policy.allow(capability, "workspace/**");
    }
    policy.allow("mutation.patch", "workspace/a.rs");
    context.policy = policy;
    let before = snapshot(&fixture.root);
    let error = engine
        .prepare_authorized(&context, MutationBatchId::generate(), &specs())
        .expect_err("unresolved ask");
    assert!(
        error.to_string().contains("approval required"),
        "the ask stays unresolved: {error}"
    );
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "an unresolved ask writes nothing"
    );
    assert!(journaled_batches(&fixture).is_empty());
}

#[test]
fn prepare_refuses_alias_escape_and_unnormalized_paths_with_no_effect() {
    for path in [
        "../outside.rs",
        "sub/./b.rs",
        "sub//b.rs",
        "sub\\b.rs",
        "/abs.rs",
        "",
        ".",
    ] {
        let fixture = Fixture::new();
        let engine = fixture.engine();
        let before = snapshot(&fixture.root);
        let error = engine
            .prepare_authorized(
                &fixture.context(),
                MutationBatchId::generate(),
                &[spec(path)],
            )
            .expect_err("alias or escape");
        assert!(
            matches!(error, MutationError::InvalidPath(_)),
            "{path}: {error}"
        );
        assert_eq!(snapshot(&fixture.root), before, "{path}");
        assert!(journaled_batches(&fixture).is_empty(), "{path}");
    }
}

#[cfg(unix)]
#[test]
fn prepare_refuses_symlinked_sources_and_ancestors_with_no_effect() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let engine = fixture.engine();
    symlink(fixture.ws.join("a.rs"), fixture.ws.join("link.rs")).expect("source symlink");
    symlink(fixture.ws.join("sub"), fixture.ws.join("dirlink")).expect("directory symlink");
    for path in ["link.rs", "dirlink/b.rs"] {
        let before = snapshot(&fixture.root);
        let error = engine
            .prepare_authorized(
                &fixture.context(),
                MutationBatchId::generate(),
                &[spec(path)],
            )
            .expect_err("symlink");
        assert!(
            matches!(error, MutationError::InvalidPath(_)),
            "{path}: {error}"
        );
        assert_eq!(snapshot(&fixture.root), before, "{path}");
        assert!(journaled_batches(&fixture).is_empty(), "{path}");
    }
}

#[test]
fn prepare_refuses_state_overlap_in_both_directions() {
    // State inside the workspace.
    let fixture = Fixture::new();
    let nested = MutationEngine::open(&fixture.ws, &fixture.ws.join(".state")).expect("engine");
    let before = snapshot(&fixture.root);
    let error = nested
        .prepare_authorized(&fixture.context(), MutationBatchId::generate(), &specs())
        .expect_err("nested state");
    assert!(error.to_string().contains("disjoint"), "{error}");
    assert_eq!(snapshot(&fixture.root), before);

    // Workspace inside the state directory.
    let fixture = Fixture::new();
    let enclosing = MutationEngine::open(&fixture.ws, &fixture.root).expect("engine");
    let before = snapshot(&fixture.root);
    let error = enclosing
        .prepare_authorized(&fixture.context(), MutationBatchId::generate(), &specs())
        .expect_err("enclosing state");
    assert!(error.to_string().contains("disjoint"), "{error}");
    assert_eq!(snapshot(&fixture.root), before);
}

#[test]
fn prepare_refuses_stale_preimages_without_staging_anything() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    for (path, base_hash) in [
        ("a.rs", Some(blake3_hex(b"other"))),
        ("a.rs", None),
        ("missing.rs", Some(blake3_hex(BEFORE))),
    ] {
        let before = snapshot(&fixture.root);
        let error = engine
            .prepare_authorized(
                &fixture.context(),
                MutationBatchId::generate(),
                &[PatchSpec {
                    path: path.to_owned(),
                    base_hash,
                    new_content: AFTER.to_vec(),
                }],
            )
            .expect_err("stale preimage");
        assert!(
            matches!(error, MutationError::StalePreimage { .. }),
            "{path}: {error}"
        );
        assert_eq!(snapshot(&fixture.root), before, "{path}");
        assert!(journaled_batches(&fixture).is_empty(), "{path}");
    }
}

#[test]
fn prepare_refuses_foreign_temp_collisions_and_reused_identity() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    let id = MutationBatchId::generate();
    // A foreign occupant of the exact derived temp path — with content that
    // matches the intended postimage — is never adopted as this batch's temp.
    let foreign = fixture
        .ws
        .join("sub")
        .join(format!(".b.rs.tachyon-tmp-{id}"));
    std::fs::write(&foreign, AFTER).expect("foreign temp");
    let before = snapshot(&fixture.root);
    let error = engine
        .prepare_authorized(&context, id, &specs())
        .expect_err("foreign temp collision");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(snapshot(&fixture.root), before, "the foreign temp survives");
    assert_eq!(std::fs::read(&foreign).expect("foreign temp"), AFTER);
    assert!(journaled_batches(&fixture).is_empty());
    std::fs::remove_file(&foreign).expect("remove foreign temp");

    // Reusing a reserved identity must not truncate or adopt the owned temp,
    // nor rewrite the journaled plan.
    let first = engine
        .prepare_authorized(&context, id, &specs())
        .expect("first preparation");
    let temp_a = fixture.ws.join(&first.files[0].temp_name);
    assert_eq!(std::fs::read(&temp_a).expect("owned temp"), AFTER);
    let mut rewritten = specs();
    rewritten[0].new_content = b"replaced".to_vec();
    let error = engine
        .prepare_authorized(&context, id, &rewritten)
        .expect_err("reused identity");
    assert!(error.to_string().contains("already journaled"), "{error}");
    assert_eq!(
        std::fs::read(&temp_a).expect("owned temp"),
        AFTER,
        "the owned temp is never truncated"
    );
    assert_eq!(
        std::fs::read(fixture.ws.join("sub/b.rs")).expect("source"),
        BEFORE,
        "no source is written"
    );
    let journaled = journaled_batches(&fixture);
    assert_eq!(journaled.len(), 1, "one batch, one plan");
    assert_eq!(
        journaled[&id].files[0].post_hash,
        blake3_hex(AFTER),
        "the journaled plan is unchanged"
    );
}

#[test]
fn prepare_refuses_sibling_and_malformed_journal_records() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    fixture.write("other.rs", BEFORE);
    engine
        .prepare(&[PatchSpec {
            path: "other.rs".to_owned(),
            base_hash: Some(blake3_hex(BEFORE)),
            new_content: AFTER.to_vec(),
        }])
        .expect("trusted sibling preparation");
    let with_sibling = snapshot(&fixture.root);
    let error = engine
        .prepare_authorized(&context, MutationBatchId::generate(), &specs())
        .expect_err("sibling batch");
    assert!(matches!(error, MutationError::JournalCorrupt(_)), "{error}");
    assert_eq!(
        snapshot(&fixture.root),
        with_sibling,
        "siblings are never adopted"
    );
    assert_eq!(journaled_batches(&fixture).len(), 1);

    // A torn tail is preserved evidence: never repaired to let a write proceed.
    let torn = "{\"record\": \"batch_complet";
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(fixture.state.join("mutation.log"))
        .expect("journal");
    write!(file, "{torn}").expect("torn record");
    file.sync_all().expect("sync");
    drop(file);
    let before = snapshot(&fixture.root);
    let error = engine
        .prepare_authorized(&context, MutationBatchId::generate(), &specs())
        .expect_err("torn tail");
    assert!(matches!(error, MutationError::JournalCorrupt(_)), "{error}");
    assert_eq!(snapshot(&fixture.root), before);
    assert!(
        std::fs::read_to_string(fixture.state.join("mutation.log"))
            .expect("journal")
            .ends_with(torn),
        "the torn tail is preserved"
    );
}

#[test]
fn prepare_authorizations_bind_identity_paths_and_hashes() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let id = MutationBatchId::generate();
    let ops = engine
        .prepare_authorizations(id, &specs())
        .expect("authorizations");
    // Two files: four target capabilities and three derived-temp capabilities.
    assert_eq!(ops.len(), 14);
    assert_eq!(
        ops.iter()
            .map(|op| op.capability.as_str())
            .collect::<Vec<_>>(),
        [
            "fs.metadata",
            "fs.read",
            "mutation.patch",
            "fs.write",
            "fs.metadata",
            "fs.write",
            "fs.delete",
            "fs.metadata",
            "fs.read",
            "mutation.patch",
            "fs.write",
            "fs.metadata",
            "fs.write",
            "fs.delete",
        ]
    );
    let scopes: Vec<&str> = ops.iter().map(|op| op.scope.as_str()).collect();
    assert!(scopes.contains(&"workspace/a.rs"));
    assert!(scopes.contains(&format!("workspace/sub/.b.rs.tachyon-tmp-{id}").as_str()));
    assert_eq!(scopes.len(), 14, "every scope is exact, none broad");
    for op in &ops {
        assert_eq!(
            op.operation["batch_id"],
            serde_json::json!(id),
            "the batch identity is bound"
        );
        assert_eq!(op.operation["scope"], serde_json::json!(op.scope));
        assert_eq!(op.operation["op"], serde_json::json!(op.capability));
        let plan = op.operation["plan"].as_array().expect("plan");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0]["path"], serde_json::json!("a.rs"));
        assert_eq!(plan[0]["pre_hash"], serde_json::json!(blake3_hex(BEFORE)));
        assert_eq!(plan[0]["post_hash"], serde_json::json!(blake3_hex(AFTER)));
        assert_eq!(plan[1]["path"], serde_json::json!("sub/b.rs"));
    }
}
