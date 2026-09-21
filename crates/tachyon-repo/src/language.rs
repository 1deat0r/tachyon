//! Language detection and the parser backend trait.
//!
//! [`LanguageBackend`] is the seam where Tree-sitter grammars plug in
//! later. [`HeuristicBackend`] is the initial implementation: line-based
//! symbol extraction per language, fully deterministic, zero dependencies.

use serde::{Deserialize, Serialize};

/// A symbol extracted from source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 1-based start line.
    pub start_line: u32,
    /// 1-based end line (inclusive); falls back to `start_line` when the
    /// block extent cannot be determined cheaply.
    pub end_line: u32,
}

/// Symbol kinds across supported languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Module,
    Impl,
    Constant,
    TypeAlias,
}

/// Source parsing behind a trait so Tree-sitter can replace heuristics.
pub trait LanguageBackend: Send + Sync {
    /// Extracts top-level symbols from `text` (a single file's content).
    fn extract(&self, language: crate::Language, text: &str) -> Vec<Symbol>;
}

/// Deterministic line-based extractor (initial backend, no native deps).
#[derive(Clone, Copy, Debug, Default)]
pub struct HeuristicBackend;

impl LanguageBackend for HeuristicBackend {
    fn extract(&self, language: crate::Language, text: &str) -> Vec<Symbol> {
        let lines: Vec<&str> = text.lines().collect();
        match language {
            crate::Language::Rust => extract_rust(&lines),
            crate::Language::Python => extract_python(&lines),
            crate::Language::TypeScript | crate::Language::JavaScript => {
                extract_braced(&lines, TS_PATTERNS)
            }
            crate::Language::Unknown => Vec::new(),
        }
    }
}

struct Pattern {
    kind: SymbolKind,
    /// Literal prefixes to try (after trimming leading whitespace).
    prefixes: &'static [&'static str],
    /// If true, the name follows the prefix after skipping generics/
    /// qualifiers up to the first identifier.
    skip_qualifiers: bool,
}

const RUST_PATTERNS: &[Pattern] = &[
    Pattern {
        kind: SymbolKind::Function,
        prefixes: &["fn "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Struct,
        prefixes: &["struct "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Enum,
        prefixes: &["enum "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Trait,
        prefixes: &["trait "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Module,
        prefixes: &["mod "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Constant,
        prefixes: &["const ", "static "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Impl,
        prefixes: &["impl "],
        skip_qualifiers: true,
    },
];

const TS_PATTERNS: &[Pattern] = &[
    Pattern {
        kind: SymbolKind::Function,
        prefixes: &["function ", "async function "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Class,
        prefixes: &["class ", "abstract class "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Interface,
        prefixes: &["interface "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::Enum,
        prefixes: &["enum ", "const enum "],
        skip_qualifiers: false,
    },
    Pattern {
        kind: SymbolKind::TypeAlias,
        prefixes: &["type "],
        skip_qualifiers: false,
    },
];

/// Strips Rust visibility/qualifier prefixes (`pub(...)`, `async`, `unsafe`,
/// `extern`, `default`) so matching starts at the item keyword.
fn strip_rust_qualifiers(line: &str) -> &str {
    let mut rest = line.trim_start();
    loop {
        let next = rest
            .strip_prefix("pub(crate)")
            .or_else(|| rest.strip_prefix("pub(super)"))
            .or_else(|| rest.strip_prefix("pub(self)"))
            .or_else(|| rest.strip_prefix("pub "))
            .or_else(|| rest.strip_prefix("async "))
            .or_else(|| rest.strip_prefix("unsafe "))
            .or_else(|| rest.strip_prefix("const "))
            .or_else(|| rest.strip_prefix("default "));
        if let Some(stripped) = next {
            rest = stripped.trim_start();
        } else if rest.starts_with("pub(")
            && let Some(end) = rest.find(')')
        {
            rest = rest[end + 1..].trim_start();
        } else {
            return rest;
        }
    }
}

fn first_identifier(text: &str) -> Option<&str> {
    let start = text.find(|c: char| c.is_alphabetic() || c == '_')?;
    let end = text[start..]
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(text.len(), |offset| start + offset);
    Some(&text[start..end])
}

/// For `impl` blocks: skips generics/where-clauses to the target type.
fn impl_target(text: &str) -> Option<&str> {
    // `impl<T> Foo<T> where ...` or `impl Trait for Foo`: take the last
    // identifier chain head before `{`/`where`/`;`.
    let head = text.split(['{', ';']).next()?.split("where").next()?;
    let after_for = head.rsplit(" for ").next().unwrap_or(head);
    // Drop leading `<...>` generics.
    let mut rest = after_for.trim();
    if rest.starts_with('<') {
        let mut depth = 0;
        for (index, char) in rest.char_indices() {
            match char {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        rest = rest[index + 1..].trim_start();
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    first_identifier(rest)
}

fn match_pattern(line: &str, pattern: &Pattern) -> Option<String> {
    for prefix in pattern.prefixes {
        if let Some(rest) = line.strip_prefix(prefix) {
            if pattern.skip_qualifiers {
                return impl_target(rest).map(str::to_owned);
            }
            // `type X =` / `const X:` / `mod x;` — identifier first.
            if let Some(name) = first_identifier(rest) {
                return Some(name.to_owned());
            }
        }
    }
    None
}

fn extract_rust(lines: &[&str]) -> Vec<Symbol> {
    extract_braced_with(lines, RUST_PATTERNS, strip_rust_qualifiers)
}

fn extract_braced(lines: &[&str], patterns: &[Pattern]) -> Vec<Symbol> {
    extract_braced_with(lines, patterns, |line| {
        line.trim_start()
            .strip_prefix("export ")
            .unwrap_or_else(|| line.trim_start())
            .strip_prefix("default ")
            .unwrap_or_else(|| {
                line.trim_start()
                    .strip_prefix("export ")
                    .unwrap_or_else(|| line.trim_start())
            })
            .trim_start()
    })
}

fn extract_braced_with(
    lines: &[&str],
    patterns: &[Pattern],
    normalize: impl Fn(&str) -> &str,
) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let normalized = normalize(line);
        for pattern in patterns {
            if let Some(name) = match_pattern(normalized, pattern) {
                let start = crate::line_no(index);
                let end = brace_extent(lines, index);
                symbols.push(Symbol {
                    name,
                    kind: pattern.kind,
                    start_line: start,
                    end_line: end,
                });
                break;
            }
        }
    }
    symbols
}

/// Cheap block extent: from `index`, scan brace depth until it returns to
/// the entry level (or EOF). Ignores braces in strings/comments — extent is
/// advisory, identity comes from hashes.
fn brace_extent(lines: &[&str], index: usize) -> u32 {
    let mut depth: i32 = 0;
    let mut seen_open = false;
    for (offset, line) in lines.iter().enumerate().skip(index) {
        for char in line.chars() {
            match char {
                '{' => {
                    depth += 1;
                    seen_open = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if seen_open && depth <= 0 {
            return crate::line_no(offset);
        }
    }
    crate::line_count(lines.len())
}

/// Python: indent-scoped `def`/`class`.
fn extract_python(lines: &[&str]) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some((kind, rest)) = trimmed
            .strip_prefix("async def ")
            .map(|rest| (SymbolKind::Function, rest))
            .or_else(|| {
                trimmed
                    .strip_prefix("def ")
                    .map(|rest| (SymbolKind::Function, rest))
            })
            .or_else(|| {
                trimmed
                    .strip_prefix("class ")
                    .map(|rest| (SymbolKind::Class, rest))
            })
        else {
            continue;
        };
        let Some(name) = first_identifier(rest) else {
            continue;
        };
        let indent = line.len() - trimmed.len();
        let start = crate::line_no(index);
        let mut end = crate::line_count(lines.len());
        for (offset, follower) in lines.iter().enumerate().skip(index + 1) {
            let follower_trimmed = follower.trim_start();
            if follower_trimmed.is_empty() {
                continue;
            }
            if follower.len() - follower_trimmed.len() <= indent {
                end = crate::line_no(offset).saturating_sub(1);
                break;
            }
        }
        symbols.push(Symbol {
            name: name.to_owned(),
            kind,
            start_line: start,
            end_line: end,
        });
    }
    symbols
}
