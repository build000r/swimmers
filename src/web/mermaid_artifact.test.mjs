import test from "node:test";
import assert from "node:assert/strict";

import {
  MERMAID_PLAN_FILES_MAX,
  boundedArtifactText,
  buildMermaidArtifactView,
  isSafeMermaidPlanFileName,
  mermaidPlanTabClickPlan,
  planFileLabel,
  sanitizeMermaidPlanFiles,
} from "./mermaid_artifact.js";

test("boundedArtifactText appends a truncation marker without changing short text", () => {
  assert.deepEqual(boundedArtifactText("short", 12, "truncated"), {
    text: "short",
    truncated: false,
  });

  assert.deepEqual(boundedArtifactText("abcdefgh", 4, "truncated"), {
    text: "abcd\n\n[truncated]",
    truncated: true,
  });
});

test("Mermaid plan file sanitization rejects path-ish names, dedupes, and caps", () => {
  const safeNames = Array.from({ length: MERMAID_PLAN_FILES_MAX + 2 }, (_, index) => `plan-${index}.mmdx`);
  const result = sanitizeMermaidPlanFiles([
    " overview.mmdx ",
    "overview.mmdx",
    "../secret.md",
    "nested/path.mmdx",
    ".",
    ...safeNames,
  ]);

  assert.equal(isSafeMermaidPlanFileName("overview.mmdx"), true);
  assert.equal(isSafeMermaidPlanFileName("../secret.md"), false);
  assert.equal(result.files.length, MERMAID_PLAN_FILES_MAX);
  assert.deepEqual(result.files.slice(0, 2), ["overview.mmdx", "plan-0.mmdx"]);
  assert.equal(result.hiddenCount, 3);
  assert.equal(result.cappedCount, 3);
});

test("mermaidPlanTabClickPlan preserves target matching and dataset forwarding", () => {
  const tabFor = (dataset) => {
    const button = { dataset };
    return {
      button,
      target: {
        closest(selector) {
          return selector === "button[data-plan-file]" ? button : null;
        },
      },
    };
  };

  assert.deepEqual(mermaidPlanTabClickPlan("keydown", tabFor({ planFile: "overview.mmdx" }).target), {
    type: "ignore",
  });
  assert.deepEqual(mermaidPlanTabClickPlan("click", null), {
    type: "ignore",
  });
  assert.deepEqual(mermaidPlanTabClickPlan("click", { closest: () => null }), {
    type: "ignore",
  });
  assert.deepEqual(mermaidPlanTabClickPlan("click", tabFor({ planFile: " overview.mmdx " }).target), {
    type: "load_plan_file",
    planFile: " overview.mmdx ",
  });
  assert.deepEqual(mermaidPlanTabClickPlan("click", tabFor({}).target), {
    type: "load_plan_file",
    planFile: undefined,
  });
});

test("buildMermaidArtifactView produces source, plan files, and status text", () => {
  const view = buildMermaidArtifactView(
    {
      available: true,
      path: "/tmp/demo.mmdx",
      updated_at: "2026-06-03T00:00:00Z",
      source: "flowchart TD",
      plan_files: ["alpha-plan.mmdx", "../unsafe.mmdx"],
      error: "render warning",
    },
    {
      formatTime: (value) => `formatted ${value}`,
    },
  );

  assert.equal(view.available, true);
  assert.equal(view.source, "flowchart TD");
  assert.deepEqual(view.planFiles, ["alpha-plan.mmdx"]);
  assert.match(view.status, /available: true/);
  assert.match(view.status, /updated: formatted 2026-06-03T00:00:00Z/);
  assert.match(view.status, /plan files: 1 unsafe name hidden/);
  assert.match(view.status, /error: render warning/);
  assert.equal(planFileLabel("alpha-plan.mmdx"), "alpha plan");
});
