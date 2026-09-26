//! Milestone 4 gate + Vertical Slice A: "Where is refreshToken defined
//! and used?" — zero LLM calls, structured locations, fast on fixtures.

use std::path::PathBuf;
use tachyon_repo::language::HeuristicBackend;
use tachyon_repo::{
    Inventory, SearchOptions, SymbolIndex, TextProjection, Watcher, lexical_search,
};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("tachyon-m4-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/auth.ts"),
        "export function refreshToken(session: string): string {\n  return session + ':refreshed';\n}\n\nexport class SessionStore {\n  token = refreshToken('seed');\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/login.ts"),
        "import { refreshToken } from './auth';\n\nexport function login() {\n  return refreshToken('new');\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/store.py"),
        "def save_refresh_token(value):\n    return {'refresh_token': value}\n",
    )
    .unwrap();
    std::fs::write(root.join("README.md"), "# demo\n").unwrap();
    root
}

#[test]
fn slice_a_definition_and_use() {
    let root = fixture("slice");
    let started = std::time::Instant::now();
    let inventory = Inventory::scan(&root, 10_000).unwrap();
    let mut index = SymbolIndex::new(&root, HeuristicBackend);
    index.build(&inventory);
    let answer = index.definition_use("refreshToken");
    let elapsed = started.elapsed();
    assert_eq!(answer.definitions.len(), 1);
    assert_eq!(answer.definitions[0].file, "src/auth.ts");
    assert_eq!(answer.definitions[0].line, 1);
    // Uses: import + call in login.ts, field init in auth.ts.
    let use_files: Vec<&str> = answer
        .references
        .iter()
        .map(|location| location.file.as_str())
        .collect();
    assert!(use_files.contains(&"src/login.ts"), "{use_files:?}");
    assert!(use_files.contains(&"src/auth.ts"), "{use_files:?}");
    // `refresh_token` (snake) must not match `refreshToken` (camel).
    assert!(
        !answer
            .references
            .iter()
            .any(|location| location.file == "src/store.py"),
        "{:?}",
        answer.references
    );
    assert!(elapsed.as_millis() < 2000, "slice took {elapsed:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn inventory_hashes_and_pruning() {
    let root = fixture("inventory");
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("target/debug/blob"), b"junk").unwrap();
    let inventory = Inventory::scan(&root, 10_000).unwrap();
    assert_eq!(inventory.files.len(), 4);
    let first = inventory.get("src/auth.ts").unwrap();
    assert_eq!(first.hash.len(), 64);
    // Stable across rescans.
    let again = Inventory::scan(&root, 10_000).unwrap();
    assert_eq!(again.get("src/auth.ts").unwrap().hash, first.hash);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn freshness_detects_drift() {
    let root = fixture("freshness");
    let before = Inventory::scan(&root, 10_000).unwrap();
    let mut index = SymbolIndex::new(&root, HeuristicBackend);
    index.build(&before);
    assert!(index.verify(&before).is_empty());
    // Mutate one file: new inventory disagrees with the index.
    std::fs::write(root.join("src/login.ts"), "// changed\n").unwrap();
    let after = Inventory::scan(&root, 10_000).unwrap();
    let stale = index.verify(&after);
    assert_eq!(stale, vec!["src/login.ts".to_owned()]);
    // Refresh repairs the index at a new generation.
    let generation = index.generation;
    index.refresh(&after, &["src/login.ts"]);
    assert!(index.generation > generation);
    assert!(index.verify(&after).is_empty());
    let answer = index.definition_use("refreshToken");
    assert!(
        answer
            .definitions
            .iter()
            .any(|location| location.file == "src/auth.ts")
    );
    // login.ts no longer mentions it; the auth.ts self-use remains.
    assert!(
        !answer
            .references
            .iter()
            .any(|location| location.file == "src/login.ts"),
        "{:?}",
        answer.references
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn python_symbols_extracted() {
    let root = fixture("python");
    let inventory = Inventory::scan(&root, 10_000).unwrap();
    let mut index = SymbolIndex::new(&root, HeuristicBackend);
    index.build(&inventory);
    let definitions = index.definitions("save_refresh_token");
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].location.file, "src/store.py");
    assert_eq!(definitions[0].location.line, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lexical_search_structured() {
    let root = fixture("search");
    let inventory = Inventory::scan(&root, 10_000).unwrap();
    let projection = TextProjection::new();
    let options = SearchOptions::default();
    let hits = lexical_search(&root, &inventory, &projection, "refreshToken", &options);
    assert!(hits.len() >= 4, "{hits:?}");
    assert!(hits.iter().all(|hit| hit.column >= 1));
    let ts_only = lexical_search(
        &root,
        &inventory,
        &projection,
        "refreshToken",
        &SearchOptions {
            extensions: vec!["ts".to_owned()],
            ..SearchOptions::default()
        },
    );
    assert!(ts_only.iter().all(|hit| {
        std::path::Path::new(&hit.file)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ts"))
    }));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn watcher_reports_changes() {
    let root = fixture("watcher");
    let watcher = Watcher::watch(&root).unwrap();
    // Let the watcher settle (FSEvent on macOS runners needs longer),
    // then touch a file.
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(root.join("src/login.ts"), "// touched\n").unwrap();
    let mut seen = Vec::new();
    for _ in 0..150 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        seen.extend(watcher.pending());
        if seen.iter().any(|rel| rel == "src/login.ts") {
            break;
        }
    }
    assert!(
        seen.iter().any(|rel| rel == "src/login.ts"),
        "watcher never reported the change: {seen:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn repo_answers_real_code() {
    // Smoke over this repository itself: finds a known symbol quickly.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let started = std::time::Instant::now();
    let inventory = Inventory::scan(&workspace, 50_000).unwrap();
    assert!(!inventory.files.is_empty());
    let mut index = SymbolIndex::new(&workspace, HeuristicBackend);
    index.build(&inventory);
    let answer = index.definition_use("contain");
    assert!(
        !answer.definitions.is_empty(),
        "expected `contain` definitions"
    );
    assert!(
        answer
            .definitions
            .iter()
            .any(|location| location.file.ends_with("tachyon-policy/src/lib.rs")),
        "{:?}",
        answer.definitions
    );
    assert!(started.elapsed().as_secs() < 30, "self-index too slow");
}
