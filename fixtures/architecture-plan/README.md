# Architecture extraction fixture (Class E)

A dependency-free, nested Cargo workspace for Tachyon M14/Class E
(architecture/reasoning). Root workspace commands do not build it. Copy it to
a fresh scratch workspace before any repair; never patch this checked-in
fixture. All document text is synthetic.

The subsystem constraint: HTML rendering must live in `renderer/src/render.rs`
and `lib.rs` must only declare and re-export it. The checked-in tree violates
that constraint on purpose — `to_html` and its escape helper are inline in
`lib.rs` — so the architecture test fails while the behavior tests pass. The
extraction task moves the rendering half into the sibling `mod render;`
without changing the public API (`parse`, `to_html`, `Doc`) or any rendering
output; `catalog` is unrelated and must remain untouched.

Generate Cargo.lock offline BEFORE binding a source baseline. Broken-first
must fail for the named architecture assertions, not setup/build errors.
Correct repair changes only `renderer/src/lib.rs` and
`renderer/src/render.rs`. Tests, manifests and `catalog` are protected.
Affected verification must run `renderer`, not `catalog`.

A scripted model may replay that extraction through the real supervisor. Such
a run proves runtime integration, constrained mutation and verification, NOT
model design ability.
