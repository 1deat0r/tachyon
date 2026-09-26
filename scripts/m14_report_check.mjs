#!/usr/bin/env node
// M14 report checker (gate G8).
//
// MVP_REPORT.md must exist with every section docs/04 M14 requires, a
// disposition row for each of the eleven spec §45 MVP exit conditions,
// and figures that trace back to docs/milestones/M14_MATRIX.json (the
// artifact gate G5 produced) — so fabricated or stale numbers fail here
// instead of passing a substring check (M13 finding F4).
"use strict";

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const reportPath = path.join(root, "MVP_REPORT.md");
const matrixPath = path.join(root, "docs", "milestones", "M14_MATRIX.json");

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (!fs.existsSync(reportPath)) fail("missing MVP_REPORT.md");
if (!fs.existsSync(matrixPath)) fail("missing docs/milestones/M14_MATRIX.json (run G5 first)");
const report = fs.readFileSync(reportPath, "utf8");
const matrix = JSON.parse(fs.readFileSync(matrixPath, "utf8"));

const sections = [
  "## Verified-success comparison",
  "## Median and p95 TTFR and completion",
  "## Critical-path breakdown",
  "## Model, Jev and tool calls",
  "## Known limitations",
  "## Deferred work",
  "## Kill criteria",
  "## Spec §45 MVP exit dispositions",
];
for (const section of sections) {
  if (!report.includes(section)) fail(`missing section: ${section}`);
}

// ---- §45 exit conditions: every bullet quoted with a disposition ---------
const EXIT_CONDITIONS = [
  "CLI and TUI are usable gateway clients",
  "local gateway persists across client disconnects",
  "simple repository questions commonly use zero LLM calls",
  "complex tasks start evidence work in parallel",
  "model and judgment providers are replaceable",
  "tasks recover after process restart",
  "local mutation batches recover safely",
  "workspace containment survives traversal/symlink tests",
  "verification gates completion",
  "benchmarks report p50/p95 and verified success",
  "tachyon-full beats the in-tree serial reference",
];
const DISPOSITION = /\|\s*(MET|PARTIAL|NOT MET)\s*\|/;
for (const condition of EXIT_CONDITIONS) {
  const line = report.split("\n").find((l) => l.includes(condition));
  if (!line) fail(`§45 condition not quoted: "${condition}"`);
  if (!DISPOSITION.test(line)) {
    fail(`§45 condition lacks a MET/PARTIAL/NOT MET disposition: "${condition}"`);
  }
}
// The MVP claim row must be honest about the measured comparison.
const claimLine = report.split("\n").find((l) => l.includes("tachyon-full beats the in-tree serial reference"));
if (claimLine && claimLine.includes("| MET") && matrix.comparisons) {
  for (const [fixture, cmp] of Object.entries(matrix.comparisons)) {
    if (cmp.full_vs_reference.verified_full < cmp.full_vs_reference.verified_reference) {
      fail(`claim marked MET but ${fixture} full verified below reference`);
    }
  }
}

// ---- numbers must trace to the matrix artifact ---------------------------
// Scoping (M13 F4): cell figures must appear inside the median/p95
// section, not anywhere in the file. Leg figures live one section down
// (Critical-path breakdown), so the scope covers both sections.
const medianSection = report.split("## Median and p95 TTFR and completion")[1];
if (!medianSection) fail("median/p95 section unreadable");
const medianBody = medianSection.split("## Model, Jev and tool calls")[0];

function requireFigure(label, needle) {
  if (!medianBody.includes(String(needle))) {
    fail(`figure not in the median/p95 section for ${label}: ${needle}`);
  }
}

const totalCellSamples =
  matrix.samples_per_cell * matrix.cells.length;
let verified = 0;
for (const cell of matrix.cells) verified += cell.verified_success_rate * cell.n;
// The verified-success total is required unconditionally: on an imperfect
// run the report must state the actual N/M, never silently skip it.
requireFigure("cell verified-success total", `${verified}/${totalCellSamples}`);

for (const cell of matrix.cells.filter((c) => c.mode === "full")) {
  const m = cell.metrics;
  requireFigure(
    `${cell.fixture} full completion p50/p95`,
    `${m.completion_ms.p50}/${m.completion_ms.p95}`
  );
  requireFigure(
    `${cell.fixture} full first-edit (TTFR) p50/p95`,
    `${m.first_edit_ms.p50}/${m.first_edit_ms.p95}`
  );
}
for (const leg of matrix.legs) {
  requireFigure(`leg ${leg.leg} p50/p95`, `${leg.p50_us}/${leg.p95_us}`);
}
// Verified-success table: the leg totals use the same wording.
requireFigure(
  "serial-reference comparison sample count",
  `n=${matrix.samples_per_cell}`
);

// Every fixture must appear in the report with its class label.
for (const cell of matrix.cells.filter((c) => c.mode === "full")) {
  if (!report.includes(cell.fixture)) fail(`fixture missing from report: ${cell.fixture}`);
}

console.log("m14 report ok");
