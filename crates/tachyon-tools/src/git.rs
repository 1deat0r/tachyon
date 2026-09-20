//! Read-only git: status, diff, log, show (spec §28, M3).
//!
//! Runs the `git` binary with an allowlisted subcommand set and fixed safe
//! flags. Anything else — including flag injection — is rejected before a
//! process spawns. Dangerous writes (push, reset --hard, …) are a policy
//! matter: the trusted workspace denies them outright (see
//! `tachyon-policy`).

use crate::{ToolError, ToolsContext, authorize, process};
use std::path::Path;

/// Allowlisted subcommands with their fixed flags.
fn safe_flags(subcommand: &str) -> Option<&'static [&'static str]> {
    match subcommand {
        "status" => Some(&["--short", "--branch", "--untracked-files=all"]),
        "diff" => Some(&["--no-color", "--no-ext-diff"]),
        "log" => Some(&["--no-color", "--oneline", "--decorate", "-n", "20"]),
        "show" => Some(&["--no-color", "--no-ext-diff", "--stat", "HEAD"]),
        "rev-parse" => Some(&["--show-toplevel"]),
        _ => None,
    }
}

/// Runs a read-only git subcommand with `cwd` contained in the workspace.
/// Extra user args are rejected: the flag set is fixed (no injection).
pub async fn git_read(
    context: &ToolsContext,
    subcommand: &str,
    cwd: &Path,
) -> Result<String, ToolError> {
    let flags = safe_flags(subcommand).ok_or_else(|| {
        ToolError::InvalidArgs(format!("git subcommand not allowlisted: {subcommand}"))
    })?;
    let (resolved_cwd, scope) = crate::resolve_scope(&context.workspace_root, cwd)?;
    let operation = serde_json::json!({
        "op": "git.read",
        "subcommand": subcommand,
        "scope": scope,
    });
    authorize(
        &context.policy,
        &context.approvals,
        "git.read",
        &scope,
        &operation,
        &format!("git {subcommand} in {}", resolved_cwd.display()),
    )?;
    let mut spec = process::ProcessSpec::new("git");
    spec.args = vec!["--no-pager".to_owned(), subcommand.to_owned()];
    spec.args
        .extend(flags.iter().map(|flag| (*flag).to_owned()));
    spec.cwd = Some(resolved_cwd);
    let receipt = process::run(context, &spec).await?;
    if receipt.exit_code != Some(0) {
        return Err(ToolError::Git(
            String::from_utf8_lossy(&receipt.stderr).into_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&receipt.stdout).into_owned())
}
