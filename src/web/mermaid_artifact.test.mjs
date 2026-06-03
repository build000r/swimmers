import test from "node:test";
import assert from "node:assert/strict";

import {
  MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS,
  MERMAID_PLAN_FILES_MAX,
  boundedArtifactText,
  buildMermaidArtifactView,
  isSafeMermaidPlanFileName,
  loadMermaidPlanFileWithRuntime,
  mermaidPlanTabClickPlan,
  planFileLabel,
  sanitizeMermaidPlanFiles,
} from "./mermaid_artifact.js";
import {
  createMermaidArtifactController,
} from "./mermaid_artifact_controller.js";

function createClassList(initial = []) {
  const classes = new Set(initial);
  return {
    add(...names) {
      for (const name of names) classes.add(name);
    },
    remove(...names) {
      for (const name of names) classes.delete(name);
    },
    toggle(name, force) {
      const enabled = force === undefined ? !classes.has(name) : Boolean(force);
      if (enabled) {
        classes.add(name);
      } else {
        classes.delete(name);
      }
      return enabled;
    },
    contains(name) {
      return classes.has(name);
    },
  };
}

function createMockElement(id) {
  return {
    id,
    type: "",
    className: "",
    dataset: {},
    textContent: "",
    innerHTML: "",
    children: [],
    classList: createClassList(),
    appendChild(child) {
      this.children.push(child);
      return child;
    },
  };
}

function createPlanFileRuntime(options = {}) {
  const calls = [];
  const artifact = {
    planFiles: options.planFiles ?? ["plan.md"],
    activePlanFile: options.activePlanFile ?? "",
    planContent: options.planContent ?? "stale",
  };
  const content = {
    textContent: options.textContent ?? "stale content",
    classList: createClassList(options.classes ?? ["hidden", "error"]),
  };
  const session = Object.prototype.hasOwnProperty.call(options, "session")
    ? options.session
    : { session_id: "sess/0" };
  const payload = Object.prototype.hasOwnProperty.call(options, "payload")
    ? options.payload
    : { content: "loaded plan" };
  const runtime = {
    mermaidArtifact: artifact,
    mermaidPlanContent: content,
    currentSession: () => {
      calls.push(["currentSession"]);
      return session;
    },
    renderMermaidPlanTabs: () => calls.push(["renderTabs", artifact.activePlanFile, artifact.planContent]),
    setMermaidStatus: (message, isError = false) => calls.push(["status", message, Boolean(isError)]),
    syncSheetActionAvailability: () => calls.push(["sync"]),
    apiMaybeFetch: async (path) => {
      calls.push(["fetch", path]);
      return { path };
    },
    responseJsonOrNull: async (response) => {
      calls.push(["json", response?.path]);
      return payload;
    },
    locationOrigin: options.locationOrigin ?? "http://swimmers.test",
  };
  return { artifact, calls, content, runtime };
}

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

test("loadMermaidPlanFileWithRuntime no-ops without a session or trimmed file name", async () => {
  const withoutSession = createPlanFileRuntime({ session: null });
  await loadMermaidPlanFileWithRuntime("plan.md", withoutSession.runtime);
  assert.deepEqual(withoutSession.calls, [["currentSession"]]);
  assert.equal(withoutSession.artifact.planContent, "stale");
  assert.equal(withoutSession.content.textContent, "stale content");

  const withoutName = createPlanFileRuntime();
  await loadMermaidPlanFileWithRuntime("   ", withoutName.runtime);
  assert.deepEqual(withoutName.calls, [["currentSession"]]);
  assert.equal(withoutName.artifact.planContent, "stale");
  assert.equal(withoutName.content.textContent, "stale content");
});

test("loadMermaidPlanFileWithRuntime rejects unsafe or unlisted names without fetching", async () => {
  for (const fileName of ["../secret.txt", "notes.md"]) {
    const env = createPlanFileRuntime({ planFiles: ["plan.md"] });
    await loadMermaidPlanFileWithRuntime(fileName, env.runtime);

    const message = `Plan file name not allowed: ${fileName}`;
    assert.equal(env.calls.some(([kind]) => kind === "fetch"), false);
    assert.equal(env.calls.some(([kind]) => kind === "json"), false);
    assert.equal(env.artifact.planContent, "");
    assert.equal(env.content.textContent, message);
    assert.equal(env.content.classList.contains("hidden"), false);
    assert.equal(env.content.classList.contains("error"), true);
    assert.deepEqual(env.calls.slice(-2), [["status", message, true], ["sync"]]);
  }
});

test("loadMermaidPlanFileWithRuntime loads valid files with exact path and truncation status", async () => {
  const hugeContent = "x".repeat(MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS + 4096);
  const env = createPlanFileRuntime({ payload: { content: hugeContent } });
  env.runtime.apiMaybeFetch = async (path) => {
    env.calls.push(["fetch", path]);
    assert.equal(path, "/v1/sessions/sess%2F0/plan-file?name=plan.md");
    assert.equal(env.artifact.activePlanFile, "plan.md");
    assert.equal(env.artifact.planContent, "");
    assert.equal(env.content.textContent, "Loading plan file...");
    assert.equal(env.content.classList.contains("hidden"), false);
    assert.deepEqual(env.calls.slice(0, 2), [["currentSession"], ["renderTabs", "plan.md", ""]]);
    return { path };
  };

  await loadMermaidPlanFileWithRuntime(" plan.md ", env.runtime);

  assert.equal(env.artifact.planContent.length < hugeContent.length, true);
  assert.match(env.content.textContent, /Plan file truncated after 128 KiB for browser display\./);
  assert.equal(env.content.classList.contains("error"), false);
  assert.deepEqual(env.calls.slice(-2), [
    ["status", "Plan file loaded: plan.md (truncated to 128 KiB for browser display)", false],
    ["sync"],
  ]);
});

test("loadMermaidPlanFileWithRuntime preserves payload error status copy", async () => {
  const env = createPlanFileRuntime({ payload: { error: "not readable" } });
  await loadMermaidPlanFileWithRuntime("plan.md", env.runtime);

  assert.equal(env.artifact.planContent, "");
  assert.equal(env.content.textContent, "not readable");
  assert.equal(env.content.classList.contains("error"), true);
  assert.deepEqual(env.calls.slice(-2), [
    ["status", "Plan file plan.md: not readable", false],
    ["sync"],
  ]);
});

test("loadMermaidPlanFileWithRuntime reports fetch failures and syncs in finally", async () => {
  const env = createPlanFileRuntime();
  env.runtime.apiMaybeFetch = async (path) => {
    env.calls.push(["fetch", path]);
    throw new Error("network down");
  };

  await loadMermaidPlanFileWithRuntime("plan.md", env.runtime);

  assert.equal(env.artifact.planContent, "");
  assert.equal(env.content.textContent, "Failed to load plan.md: network down");
  assert.equal(env.content.classList.contains("error"), true);
  assert.equal(env.calls.some(([kind]) => kind === "json"), false);
  assert.deepEqual(env.calls.slice(-2), [
    ["status", "Failed to load plan file: network down", true],
    ["sync"],
  ]);
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

test("Mermaid artifact controller refresh owns blob lifecycle and DOM preview", async () => {
  const calls = [];
  const state = {
    mermaidArtifact: {
      loading: false,
      sessionId: null,
      artifact: null,
      svgUrl: "blob:old",
      source: "stale",
      planFiles: [],
      activePlanFile: "",
      planContent: "",
      status: "",
      error: "",
    },
  };
  const el = {
    mermaidSummary: createMockElement("summary"),
    mermaidPreview: createMockElement("preview"),
    mermaidSource: createMockElement("source"),
    mermaidPlanTabs: createMockElement("tabs"),
    mermaidPlanContent: createMockElement("content"),
  };
  const controller = createMermaidArtifactController({
    state,
    el,
    currentSession: () => ({ session_id: "sess/0" }),
    apiFetch: async () => {
      throw new Error("not used");
    },
    apiMaybeFetch: async (path) => {
      calls.push(["fetch", path]);
      if (path.endsWith("/mermaid-artifact")) {
        return { kind: "artifact" };
      }
      if (path.endsWith("/mermaid-artifact/svg")) {
        return {
          async blob() {
            calls.push(["blob"]);
            return { kind: "svg" };
          },
        };
      }
      return null;
    },
    responseJsonOrNull: async (response) => {
      calls.push(["json", response?.kind]);
      return {
        available: true,
        path: "/tmp/diagram.mmd",
        updated_at: "2026-06-03T00:00:00Z",
        source: "graph TD\nA-->B",
        plan_files: ["plan.md"],
      };
    },
    syncSheetActionAvailability: () => calls.push(["sync"]),
    formatTime: (value) => `formatted ${value}`,
    documentRef: { createElement: createMockElement },
    ElementClass: null,
    URLImpl: {
      revokeObjectURL: (url) => calls.push(["revoke", url]),
      createObjectURL: (blob) => {
        calls.push(["create", blob.kind]);
        return "blob:new";
      },
    },
    locationOrigin: "http://swimmers.test",
  });

  await controller.refresh();

  assert.equal(state.mermaidArtifact.loading, false);
  assert.equal(state.mermaidArtifact.sessionId, "sess/0");
  assert.equal(state.mermaidArtifact.svgUrl, "blob:new");
  assert.equal(state.mermaidArtifact.source, "graph TD\nA-->B");
  assert.deepEqual(state.mermaidArtifact.planFiles, ["plan.md"]);
  assert.match(el.mermaidSummary.textContent, /path: \/tmp\/diagram\.mmd/);
  assert.match(el.mermaidSummary.textContent, /updated: formatted 2026-06-03T00:00:00Z/);
  assert.equal(el.mermaidPreview.children[0].src, "blob:new");
  assert.equal(el.mermaidPreview.children[0].alt, "Mermaid artifact preview");
  assert.equal(el.mermaidPlanTabs.children[0].dataset.planFile, "plan.md");
  assert.deepEqual(calls, [
    ["revoke", "blob:old"],
    ["fetch", "/v1/sessions/sess%2F0/mermaid-artifact"],
    ["json", "artifact"],
    ["fetch", "/v1/sessions/sess%2F0/mermaid-artifact/svg"],
    ["blob"],
    ["create", "svg"],
    ["sync"],
    ["sync"],
  ]);
});
