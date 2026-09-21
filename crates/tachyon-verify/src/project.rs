//! Path-based project detection; Cargo, not heuristic TOML parsing, validates manifests.
//!
//! Affected selection uses manifest locations plus a reverse-dependency
//! closure over workspace `Cargo.toml` files. Dependency names come from a
//! small line-oriented reader (package name plus dependency section entries);
//! anything it cannot establish broadens conservatively to the workspace
//! check instead of silently omitting a possibly affected test.
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
    let by_name: BTreeMap<&str, &str> = packages
        .iter()
        .map(|(manifest, name)| (name.as_str(), manifest.as_str()))
        .collect();
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
    // Dependents whose names never resolved still broaden via the caller.
    let _ = by_name;
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
    let mut section = String::new();
    for raw in content.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section.clear();
            section.push_str(line[1..line.len() - 1].trim());
            // `[dependencies.foo]` style headers declare `foo` directly.
            if let Some(declared) = dependency_header(&section) {
                dependencies.insert(declared);
            }
            continue;
        }
        if !is_dependency_section(&section) {
            if section == "package"
                && let Some(value) = assignment(line, "name")
            {
                name = Some(value);
            }
            continue;
        }
        let (key, _) = line.split_once('=')?;
        let key = key.trim().trim_matches(['\'', '"']);
        if key.is_empty() {
            return None;
        }
        dependencies.insert(key.to_owned());
    }
    Some(ParsedManifest { name, dependencies })
}

fn is_dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "dev-dependencies"
        || section == "build-dependencies"
        || section.starts_with("dependencies.")
        || section.starts_with("dev-dependencies.")
        || section.starts_with("build-dependencies.")
        || section.contains(".dependencies")
}

fn dependency_header(section: &str) -> Option<String> {
    let mut parts = section.split('.').map(str::trim);
    let kind = parts.next()?;
    if kind != "dependencies" && kind != "dev-dependencies" && kind != "build-dependencies" {
        // `[target.<spec>.dependencies.<name>]` form: find the segment.
        let segments: Vec<&str> = section.split('.').map(str::trim).collect();
        let index = segments.iter().position(|part| {
            *part == "dependencies" || *part == "dev-dependencies" || *part == "build-dependencies"
        })?;
        return segments
            .get(index + 1)
            .map(|name| name.trim().trim_matches(['\'', '"']).to_owned())
            .filter(|name| !name.is_empty());
    }
    parts
        .next()
        .map(|name| name.trim_matches(['\'', '"']).to_owned())
        .filter(|name| !name.is_empty())
}

fn assignment(line: &str, key: &str) -> Option<String> {
    let (found, value) = line.split_once('=')?;
    if found.trim() != key {
        return None;
    }
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .map(str::to_owned)
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
