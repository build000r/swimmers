import { normalizePlanFileResponse } from "./contracts.js";

export const MERMAID_SOURCE_DISPLAY_MAX_CHARS = 64 * 1024;
export const MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS = 128 * 1024;
export const MERMAID_PLAN_FILES_MAX = 32;

export function boundedArtifactText(value, maxChars, marker) {
  const text = String(value || "");
  if (text.length <= maxChars) {
    return { text, truncated: false };
  }
  return {
    text: `${text.slice(0, maxChars)}\n\n[${marker}]`,
    truncated: true,
  };
}

export function isSafeMermaidPlanFileName(name) {
  const value = String(name || "").trim();
  return Boolean(
    value
      && value.length <= 96
      && value !== "."
      && value !== ".."
      && !value.includes("..")
      && /^[A-Za-z0-9._-]+$/.test(value),
  );
}

export function sanitizeMermaidPlanFiles(value, maxFiles = MERMAID_PLAN_FILES_MAX) {
  const input = Array.isArray(value) ? value : [];
  const safe = [];
  let hiddenCount = 0;
  for (const rawName of input) {
    const name = String(rawName || "").trim();
    if (!isSafeMermaidPlanFileName(name)) {
      hiddenCount += 1;
      continue;
    }
    if (!safe.includes(name)) {
      safe.push(name);
    }
  }
  const files = safe.slice(0, maxFiles);
  return {
    files,
    hiddenCount,
    cappedCount: safe.length - files.length,
  };
}

export function planFileLabel(name) {
  const stem = String(name || "").replace(/\.[^.]+$/, "");
  return stem.replace(/[-_]+/g, " ") || name;
}

export function mermaidPlanTabClickPlan(eventType, target) {
  if (eventType !== "click") {
    return { type: "ignore" };
  }
  const button = target?.closest?.("button[data-plan-file]") ?? null;
  if (!button) {
    return { type: "ignore" };
  }
  return { type: "load_plan_file", planFile: button.dataset?.planFile };
}

export async function loadMermaidPlanFileWithRuntime(name, runtime) {
  const session = runtime.currentSession();
  const fileName = String(name || "").trim();
  if (!session || !fileName) {
    return;
  }
  const artifactState = typeof runtime.mermaidArtifact === "function"
    ? runtime.mermaidArtifact()
    : runtime.mermaidArtifact;
  const contentElement = runtime.mermaidPlanContent;
  if (!isSafeMermaidPlanFileName(fileName) || !artifactState.planFiles.includes(fileName)) {
    const message = `Plan file name not allowed: ${fileName}`;
    artifactState.planContent = "";
    contentElement.classList.remove("hidden");
    contentElement.textContent = message;
    contentElement.classList.add("error");
    runtime.setMermaidStatus(message, true);
    runtime.syncSheetActionAvailability();
    return;
  }

  artifactState.activePlanFile = fileName;
  artifactState.planContent = "";
  runtime.renderMermaidPlanTabs();
  contentElement.classList.remove("hidden");
  contentElement.textContent = "Loading plan file...";
  try {
    const origin = typeof runtime.locationOrigin === "function" ? runtime.locationOrigin() : runtime.locationOrigin;
    const url = new URL(`/v1/sessions/${encodeURIComponent(session.session_id)}/plan-file`, origin);
    url.searchParams.set("name", fileName);
    const response = await runtime.apiMaybeFetch(url.pathname + url.search);
    const payload = await runtime.responseJsonOrNull(response, normalizePlanFileResponse);
    const contentResult = boundedArtifactText(
      payload?.content || "",
      MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS,
      `Plan file truncated after ${MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS / 1024} KiB for browser display.`,
    );
    artifactState.planContent = contentResult.text;
    contentElement.textContent =
      contentResult.text || payload?.error || `${fileName} is unavailable.`;
    contentElement.classList.toggle("error", Boolean(payload?.error));
    runtime.setMermaidStatus(
      payload?.error
        ? `Plan file ${fileName}: ${payload.error}`
        : contentResult.truncated
          ? `Plan file loaded: ${fileName} (truncated to ${MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS / 1024} KiB for browser display)`
          : `Plan file loaded: ${fileName}`,
    );
  } catch (error) {
    contentElement.textContent = `Failed to load ${fileName}: ${error.message}`;
    contentElement.classList.add("error");
    runtime.setMermaidStatus(`Failed to load plan file: ${error.message}`, true);
  } finally {
    runtime.syncSheetActionAvailability();
  }
}

export function buildMermaidArtifactView(payload, options = {}) {
  const sourceMaxChars = options.sourceMaxChars ?? MERMAID_SOURCE_DISPLAY_MAX_CHARS;
  const planFilesMax = options.planFilesMax ?? MERMAID_PLAN_FILES_MAX;
  const formatTime = options.formatTime ?? ((value) => String(value || ""));
  const available = Boolean(payload?.available);
  const path = payload?.path || "(unknown path)";
  const updatedAt = payload?.updated_at ? formatTime(payload.updated_at) : "unknown";
  const sourceResult = boundedArtifactText(
    payload?.source || "",
    sourceMaxChars,
    `Mermaid source truncated after ${sourceMaxChars / 1024} KiB for browser display.`,
  );
  const planFileResult = sanitizeMermaidPlanFiles(payload?.plan_files, planFilesMax);
  const planFiles = planFileResult.files;
  const statusLines = [
    `available: ${available}`,
    `path: ${path}`,
    `updated: ${updatedAt}`,
    planFiles.length ? `plan files: ${planFiles.join(", ")}` : null,
    sourceResult.truncated ? `source: truncated to ${sourceMaxChars / 1024} KiB for browser display` : null,
    planFileResult.cappedCount ? `plan files: showing first ${planFilesMax}; ${planFileResult.cappedCount} hidden` : null,
    planFileResult.hiddenCount ? `plan files: ${planFileResult.hiddenCount} unsafe name${planFileResult.hiddenCount === 1 ? "" : "s"} hidden` : null,
    payload?.error ? `error: ${payload.error}` : null,
  ].filter(Boolean);

  return {
    available,
    path,
    updatedAt,
    source: sourceResult.text,
    sourceResult,
    planFiles,
    planFileResult,
    status: statusLines.join("\n"),
    statusLines,
  };
}
