import {
  buildMermaidArtifactView,
  loadMermaidPlanFileWithRuntime,
  mermaidPlanTabClickPlan,
  planFileLabel,
} from "./mermaid_artifact.js";
import { normalizeMermaidArtifactResponse } from "./contracts.js";

export function createMermaidArtifactController({
  state,
  el,
  currentSession,
  apiFetch,
  apiMaybeFetch,
  responseJsonOrNull,
  syncSheetActionAvailability,
  formatTime = (value) => String(value || ""),
  documentRef = globalThis.document,
  ElementClass = globalThis.Element,
  URLImpl = globalThis.URL,
  locationOrigin = () => globalThis.window?.location?.origin ?? "http://localhost",
} = {}) {
  function artifactState() {
    return state.mermaidArtifact;
  }

  function setStatus(message, isError = false) {
    const artifact = artifactState();
    artifact.status = message;
    artifact.error = isError ? message : "";
    if (el.mermaidSummary) {
      el.mermaidSummary.textContent = message || "";
      el.mermaidSummary.classList.toggle("error", Boolean(isError));
    }
  }

  function renderPlanTabs() {
    const artifact = artifactState();
    const files = artifact.planFiles;
    el.mermaidPlanTabs.innerHTML = "";
    el.mermaidPlanTabs.classList.toggle("hidden", !files.length);
    if (!files.length) {
      return;
    }

    for (const name of files) {
      const button = documentRef.createElement("button");
      button.type = "button";
      button.className = "ghost-button";
      button.dataset.planFile = name;
      button.textContent = planFileLabel(name);
      button.classList.toggle("active", name === artifact.activePlanFile);
      el.mermaidPlanTabs.appendChild(button);
    }
  }

  function renderArtifact(payload) {
    const artifact = artifactState();
    artifact.artifact = payload;
    const view = buildMermaidArtifactView(payload, { formatTime });
    artifact.source = view.source;
    artifact.planFiles = view.planFiles;
    artifact.activePlanFile = "";
    artifact.planContent = "";
    el.mermaidSource.textContent = view.source || "Mermaid source unavailable.";
    el.mermaidPreview.innerHTML = "";
    el.mermaidPlanContent.textContent = "";
    el.mermaidPlanContent.classList.add("hidden");
    el.mermaidPlanContent.classList.remove("error");

    if (view.available && artifact.svgUrl) {
      const img = documentRef.createElement("img");
      img.src = artifact.svgUrl;
      img.alt = "Mermaid artifact preview";
      img.className = "mermaid-preview-image";
      el.mermaidPreview.appendChild(img);
    }

    renderPlanTabs();
    setStatus(view.status);
    syncSheetActionAvailability();
  }

  function clearSvgUrl() {
    const artifact = artifactState();
    if (artifact.svgUrl && typeof URLImpl?.revokeObjectURL === "function") {
      URLImpl.revokeObjectURL(artifact.svgUrl);
    }
    artifact.svgUrl = "";
  }

  function setSvgUrl(url) {
    clearSvgUrl();
    artifactState().svgUrl = url || "";
  }

  function eventTargetElement(target) {
    if (!target) {
      return null;
    }
    if (ElementClass) {
      return target instanceof ElementClass ? target : null;
    }
    return typeof target.closest === "function" ? target : null;
  }

  async function refresh() {
    const session = currentSession();
    if (!session) {
      return;
    }

    const sessionId = session.session_id;
    const artifact = artifactState();
    artifact.loading = true;
    artifact.sessionId = sessionId;
    artifact.artifact = null;
    clearSvgUrl();
    artifact.source = "";
    el.mermaidPreview.innerHTML = "";
    el.mermaidSource.textContent = "";
    // A newer refresh (session switch) overwrites artifact.sessionId; bail before
    // mutating state/DOM so a slow earlier response can't paint a stale artifact
    // over the newer session's.
    const superseded = () => artifact.sessionId !== sessionId;
    try {
      const artifactResponse = await apiMaybeFetch(`/v1/sessions/${encodeURIComponent(sessionId)}/mermaid-artifact`);
      const payload = await responseJsonOrNull(artifactResponse, normalizeMermaidArtifactResponse);
      if (superseded()) {
        return;
      }
      artifact.artifact = payload;
      if (payload?.available) {
        const svgResponse = await apiMaybeFetch(`/v1/sessions/${encodeURIComponent(sessionId)}/mermaid-artifact/svg`);
        if (superseded()) {
          return;
        }
        if (svgResponse) {
          const objectUrl = URLImpl.createObjectURL(await svgResponse.blob());
          if (superseded()) {
            URLImpl.revokeObjectURL(objectUrl);
            return;
          }
          setSvgUrl(objectUrl);
        } else {
          clearSvgUrl();
        }
      } else {
        clearSvgUrl();
      }
      renderArtifact(payload);
    } catch (error) {
      if (!superseded()) {
        setStatus(`Failed to load Mermaid artifact: ${error.message}`, true);
      }
    } finally {
      // Only the still-active refresh owns the loading flag.
      if (!superseded()) {
        artifact.loading = false;
        syncSheetActionAvailability();
      }
    }
  }

  async function openHost() {
    const session = currentSession();
    if (!session) {
      return;
    }

    try {
      const response = await apiFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/mermaid-artifact/open`, {
        method: "POST",
      });
      const payload = await response.json();
      setStatus(`Opened Mermaid artifact${payload?.path ? `: ${payload.path}` : ""}.`);
    } catch (error) {
      setStatus(`Failed to open Mermaid artifact: ${error.message}`, true);
    }
  }

  async function loadPlanFile(name) {
    await loadMermaidPlanFileWithRuntime(name, {
      mermaidArtifact: artifactState,
      mermaidPlanContent: el.mermaidPlanContent,
      currentSession,
      renderMermaidPlanTabs: renderPlanTabs,
      setMermaidStatus: setStatus,
      syncSheetActionAvailability,
      apiMaybeFetch,
      responseJsonOrNull,
      locationOrigin,
    });
  }

  async function handlePlanTabsClick(event) {
    const plan = mermaidPlanTabClickPlan(event?.type, eventTargetElement(event?.target));
    if (plan.type === "load_plan_file") {
      await loadPlanFile(plan.planFile);
    }
  }

  return {
    clearSvgUrl,
    handlePlanTabsClick,
    loadPlanFile,
    openHost,
    refresh,
    renderArtifact,
    renderPlanTabs,
    setStatus,
    setSvgUrl,
  };
}
