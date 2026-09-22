//! Path-based project detection; Cargo, not heuristic TOML parsing, validates manifests.
//!
//! Affected selection uses manifest locations plus a reverse-dependency
//! closure over workspace `Cargo.toml` files. The narrow metadata recognizer
//! supports flat dependency sections, simple version/path entries and inline
//! `package` renames. Inheritance, target/separate dependency tables, complex
//! values and unknown syntax explicitly append `cargo test --offline --workspace`.
//! No resolver subprocess or network access is needed to make that safe choice.
use crate::{CommandCheck, VerificationRisk, VerifyError, WorkspaceSnapshot};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Deterministic project-specific selection. No process is spawned during detection.
pub trait ProjectDetector {
    fn commands(
        &self,
        baseline: &WorkspaceSnapshot,
        current: &WorkspaceSnapshot,
        risk: VerificationRisk,
    ) -> Result<Vec<CommandCheck>, VerifyError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RustProjectDetector;

impl ProjectDetector for RustProjectDetector {
    fn commands(
        &self,
        baseline: &WorkspaceSnapshot,
        current: &WorkspaceSnapshot,
        risk: VerificationRisk,
    ) -> Result<Vec<CommandCheck>, VerifyError> {
        let manifests: BTreeSet<_> = baseline
            .files
            .keys()
            .chain(current.files.keys())
            .filter(|path| {
                Path::new(path)
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml")
            })
            .cloned()
            .collect();
        if manifests.is_empty() {
            return Ok(vec![]);
        }
        let mut affected = BTreeSet::new();
        let changed = baseline.changed_paths(current);
        let mut broad = changed.is_empty();
        for path in changed {
            let file = Path::new(&path);
            // Only ordinary Rust module sources have narrowly known impact.
            // Manifests, lockfiles, build scripts, config, data and unknown
            // shared inputs expand conservatively; Cargo resolves metadata.
            if file.extension().is_none_or(|ext| ext != "rs")
                || file.file_name().is_some_and(|name| name == "build.rs")
            {
                broad = true;
                continue;
            }
            let mut found = false;
            let mut parent = Path::new(&path).parent();
            while let Some(dir) = parent {
                let candidate = dir.join("Cargo.toml").to_string_lossy().replace('\\', "/");
                if manifests.contains(&candidate) {
                    if candidate == "Cargo.toml" {
                        broad = true;
                    } else {
                        affected.insert(candidate);
                    }
                    found = true;
                    break;
                }
                parent = dir.parent();
            }
            broad |= !found;
        }
        // Reverse-dependency closure: a change in `a` must also run tests of
        // members depending on `a`. Unknown dependency metadata broadens
        // instead of omitting possibly affected dependents.
        if !affected.is_empty() {
            match reverse_dependents(baseline, &manifests, &affected) {
                Ok(extra) => {
                    affected.extend(extra);
                }
                Err(()) => {
                    broad = true;
                }
            }
        }
        let mut commands: Vec<_> = affected
            .into_iter()
            .map(|manifest| {
                cargo(vec![
                    "test".into(),
                    "--offline".into(),
                    "--manifest-path".into(),
                    manifest,
                ])
            })
            .collect();
        if broad || risk == VerificationRisk::Full {
            commands.push(cargo(vec![
                "test".into(),
                "--offline".into(),
                "--workspace".into(),
            ]));
        }
        Ok(commands)
    }
}

/// Finds workspace members that (transitively) depend on any affected
/// manifest. Returns `Err(())` when metadata cannot be established.
fn reverse_dependents(
    baseline: &WorkspaceSnapshot,
    manifests: &BTreeSet<String>,
    affected: &BTreeSet<String>,
) -> Result<BTreeSet<String>, ()> {
    let root = baseline.root();
    let mut packages: BTreeMap<String, String> = BTreeMap::new();
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for manifest in manifests {
        let content = std::fs::read_to_string(root.join(manifest)).map_err(|_| ())?;
        let parsed = parse_manifest(&content).ok_or(())?;
        if let Some(name) = parsed.name {
            packages.insert(manifest.clone(), name);
        }
        dependencies.insert(manifest.clone(), parsed.dependencies);
    }

    let mut names: BTreeSet<&str> = BTreeSet::new();
    for manifest in affected {
        if let Some(name) = packages.get(manifest) {
            names.insert(name.as_str());
        }
    }
    // Affected manifests without a package name (virtual manifests, parse
    // gaps) cannot anchor a closure; broaden instead.
    if names.len() != affected.iter().filter(|m| *m != "Cargo.toml").count() {
        return Err(());
    }
    let mut extra = BTreeSet::new();
    let mut queue: Vec<&str> = names.into_iter().collect();
    while let Some(name) = queue.pop() {
        for (manifest, deps) in &dependencies {
            if affected.contains(manifest) || extra.contains(manifest) {
                continue;
            }
            if deps.contains(name) {
                extra.insert(manifest.clone());
                if let Some(next) = packages.get(manifest) {
                    queue.push(next.as_str());
                }
            }
        }
    }

    Ok(extra)
}

struct ParsedManifest {
    name: Option<String>,
    dependencies: BTreeSet<String>,
}

/// Minimal line-oriented Cargo.toml reader: package name plus dependency
/// table entries. Anything unusual returns `None` so the caller broadens.
fn parse_manifest(content: &str) -> Option<ParsedManifest> {
    let mut name = None;
    let mut dependencies = BTreeSet::new();
    let mut section = "";
    let mut package = false;
    let mut workspace = false;
    for raw in content.lines() {
        // Multiline strings can contain apparent headers/assignments. This is
        // not a TOML parser: refuse them rather than interpreting their text.
        if raw.contains("\"\"\"") || raw.contains("'''") {
            return None;
        }
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = line.strip_prefix('[')?.strip_suffix(']')?.trim();
            // Only flat dependency tables and known non-dependency tables are
            // understood. Unknown/quoted/dotted/target/inherited forms broaden.
            if !section.split('.').all(simple_name)
                || !(is_dependency_section(section) || non_dependency_section(section))
            {
                return None;
            }
            package |= section == "package";
            workspace |= section == "workspace";
            continue;
        }
        if section.is_empty() {
            // Root dotted assignments or inline tables may define dependencies.
            return None;
        }
        if section == "package" {
            let (key, value) = line.split_once('=')?;
            if key.trim() == "name" {
                let value = simple_string(value.trim())?;
                if name.is_some() || !simple_name(value) {
                    return None;
                }
                name = Some(value.to_owned());
            }
        } else if is_dependency_section(section) {
            let (key, value) = line.split_once('=')?;
            dependencies.insert(dependency_name(key.trim(), value.trim())?);
        }
    }
    if (package && name.is_none()) || (!package && !workspace) {
        return None;
    }
    Some(ParsedManifest { name, dependencies })
}

/// Recognize only simple version strings and flat, string-valued dependency
/// tables. In particular, the Cargo package name is not necessarily the key.
/// Features, inheritance, escaped strings and other forms require Cargo's full
/// resolver, so they deliberately fall back to workspace verification.
fn dependency_name(key: &str, value: &str) -> Option<String> {
    if !simple_name(key) {
        return None;
    }
    if simple_string(value).is_some() {
        return Some(key.to_owned());
    }
    let fields = value.strip_prefix('{')?.strip_suffix('}')?;
    let mut seen = BTreeSet::new();
    let mut name = key;
    for field in fields.split(',') {
        let (key, value) = field.trim().split_once('=')?;
        let key = key.trim();
        if !seen.insert(key) {
            return None;
        }
        let value = simple_string(value.trim())?;
        match key {
            "package" if simple_name(value) => name = value,
            "path" | "version" | "registry" | "git" | "branch" | "tag" | "rev" => {}
            _ => return None,
        }
    }
    Some(name.to_owned())
}

fn simple_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn simple_string(value: &str) -> Option<&str> {
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let inner = value.strip_prefix(quote)?.strip_suffix(quote)?;
    (!inner.contains(['\\', '\'', '"']) && !inner.chars().any(char::is_control)).then_some(inner)
}

fn is_dependency_section(section: &str) -> bool {
    matches!(
        section,
        "dependencies" | "dev-dependencies" | "build-dependencies"
    )
}

fn non_dependency_section(section: &str) -> bool {
    matches!(
        section,
        "package" | "workspace" | "workspace.package" | "features" | "lib" | "lints"
    ) || ["package.metadata", "profile", "lints", "workspace.lints"]
        .iter()
        .any(|prefix| {
            section == *prefix
                || section
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
}

fn cargo(args: Vec<String>) -> CommandCheck {
    CommandCheck {
        program: "cargo".into(),
        args,
        cwd: ".".into(),
        env: BTreeMap::new(),
        timeout_ms: 120_000,
    }
}
