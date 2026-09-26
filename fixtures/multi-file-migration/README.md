# Intentionally broken multi-file tax fixture (Class D)

A dependency-free, nested Cargo workspace for Tachyon M14/Class D (multi-file
work). Root workspace commands do not build it. Copy it to a fresh scratch
workspace before any repair; never patch this checked-in fixture. All amounts
and rates are synthetic.

`billing::apply_tax` and `reporting::tax_for` are drifted duplicates of the
same rounding rule: both truncate toward zero, so half-cent cases round down.
The regression controls the arithmetic directly; no timing luck or network is
involved. The rounding rule is half-up on cents: `(amount * bp + 5_000) /
10_000`.

Generate Cargo.lock offline BEFORE binding a source baseline. The broken
billing and reporting tests must fail for the named rounding assertions, not
setup/build errors. Correct repair changes only `billing/src/tax.rs` and
`reporting/src/tax.rs`: apply the half-up formula in each. Tests, manifests
and `audit-unrelated` are protected. Selective affected verification must run
`billing` and `reporting`, not `audit-unrelated` (unless Full risk or an
explicitly reported conservative fallback requires the workspace).

A scripted model may replay that multi-file proposal through the real
supervisor. Such a run proves runtime integration, multi-file mutation and
verification, NOT model diagnostic ability.
