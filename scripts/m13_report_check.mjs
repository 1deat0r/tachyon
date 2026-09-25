#!/usr/bin/env node
// M13 report structural checker (gate G4).
//
// Verifies docs/milestones/M13_REPORT.md exists and carries every section,
// target id, component area, and optimization identifier the M13 ledger
// requires. Structural presence only — the numbers themselves are produced
// and asserted by the perf harness (gate G3) and reconciled against the
// final G3 transcript by the manual gate G7.
"use strict";

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const reportPath = path.join(root, "docs", "milestones", "M13_REPORT.md");

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (!fs.existsSync(reportPath)) {
  fail("missing docs/milestones/M13_REPORT.md");
}
const text = fs.readFileSync(reportPath, "utf8");

const sections = [
  "## Method",
  "## Spec §43 targets",
  "## Component baselines",
  "## Profiling",
  "## Optimizations",
  "## Findings and recommendations",
  "## Observations",
  "## Limitations",
];
for (const section of sections) {
  if (!text.includes(section)) {
    fail(`missing section: ${section}`);
  }
}

// All five §43 targets measured and passing, each with its id.
for (const target of ["T1", "T2", "T3", "T4", "T5"]) {
  const line = text
    .split("\n")
    .find((row) => row.includes(target) && row.includes("PASS"));
  if (!line) {
    fail(`no PASS line for target ${target}`);
  }
}

// All eight critical-path areas from docs/04 M13 must be addressed.
const areas = [
  "routing",
  "IPC",
  "persistence",
  "indexing",
  "scheduler dispatch",
  "model wait",
  "verification",
  "process output",
];
for (const area of areas) {
  if (!text.toLowerCase().includes(area.toLowerCase())) {
    fail(`missing critical-path area: ${area}`);
  }
}

// Both measured-and-fixed hotspots must be named with their code anchors.
for (const anchor of ["wait_for_exit", "wait_finished"]) {
  if (!text.includes(anchor)) {
    fail(`missing optimization anchor: ${anchor}`);
  }
}

// Percentile discipline: report must carry both p50 and p95 figures.
if (!text.includes("p50") || !text.includes("p95")) {
  fail("report must state p50 and p95 figures");
}

// Numbers must be reconciled against the final harness transcript.
if (!text.includes("transcript")) {
  fail("report must reference the harness transcript it transcribes");
}

console.log("m13 report ok");
