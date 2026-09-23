## What does this PR do?

<!-- Problem, why this approach, what would go wrong without it. -->

## Related Issue

<!-- Link the issue this PR addresses. If none exists, create one first. -->

Fixes #

## Type of Change

- [ ] 🐛 Bug fix (non-breaking)
- [ ] ✨ New feature (non-breaking)
- [ ] 🔒 Security fix
- [ ] 📝 Documentation
- [ ] ✅ Tests
- [ ] ♻️ Refactor (no behavior change)
- [ ] 🧹 Chore / tooling

## Changes Made

<!-- File paths + one line each. -->

-

## How to Test

<!-- Red on main (if bug), then green on this PR. -->

1.
2.

## Checklist

- [ ] Conventional Commit title (`fix(scope): …`, `feat(scope): …`)
- [ ] Only changes related to this PR (no unrelated commits)
- [ ] Local gates pass:
      `cargo fmt --check && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
- [ ] Tests added or updated for behavior changes
- [ ] `Fixes #N` or explicit "no issue" rationale
- [ ] Docs / `CONTEXT.md` / ADR updated if terms or architecture changed — or N/A
- [ ] Cross-platform impact considered (ubuntu / windows / macos CI) — or N/A
