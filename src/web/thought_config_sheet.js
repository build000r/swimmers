import {
  normalizeThoughtConfigProbeResponse,
  normalizeThoughtConfigResponse,
} from "./contracts.js";

const FALLBACK_THOUGHT_BACKENDS = [
  { key: "", label: "auto" },
  { key: "openrouter", label: "openrouter" },
  { key: "grok", label: "grok" },
];

export function fallbackThoughtBackendMetadata() {
  return FALLBACK_THOUGHT_BACKENDS.map((backend) => ({
    key: backend.key,
    label: backend.label,
    model_presets_hint: backend.key === ""
      ? "auto backend uses daemon default model"
      : backend.key === "openrouter"
        ? "presets: auto  router  cached free models"
        : backend.key === "grok"
          ? "uses Grok CLI default unless a model is set"
          : "auto backend uses daemon default model",
    model_presets: backend.key === "openrouter"
      ? ["", "openrouter/free", "nvidia/nemotron-3-super-120b-a12b:free", "arcee-ai/trinity-large-preview:free"]
      : backend.key === "grok"
        ? [""]
        : [""],
  }));
}

export function normalizeBackendKey(value) {
  const key = String(value || "").trim().toLowerCase();
  if (!key) return "";
  if (key === "claude" || key === "claude-cli" || key === "claude_cli") return "grok";
  if (key === "codex" || key === "codex-cli" || key === "codex_cli") return "grok";
  return key;
}

export function normalizeThoughtModelForBackend(backend, model) {
  const key = normalizeBackendKey(backend);
  const trimmed = String(model || "").trim();
  if (!trimmed) {
    return "";
  }
  if (key === "openrouter") {
    return trimmed.includes("/") ? trimmed : "";
  }
  if (key === "grok") {
    return trimmed;
  }
  if (!key) {
    return "";
  }
  return trimmed;
}

export function thoughtBackendMetadata(thoughtConfig = {}) {
  const backends = thoughtConfig.ui?.backends;
  return Array.isArray(backends) && backends.length ? backends : fallbackThoughtBackendMetadata();
}

export function selectedThoughtBackendMetadata(thoughtConfig = {}, backendValue = thoughtConfig?.config?.backend || "") {
  const backends = thoughtBackendMetadata(thoughtConfig);
  const backend = normalizeBackendKey(backendValue);
  return backends.find((entry) => normalizeBackendKey(entry.key) === backend) ?? backends[0] ?? null;
}

export function createThoughtConfigSheetController(runtime = {}) {
  const {
    state,
    el,
    apiFetch,
    refreshSessions = async () => {},
    syncSheetActionAvailability = () => {},
    documentRef = globalThis.document,
  } = runtime;

  function setResult(message, isError = false) {
    state.thoughtConfig.result = message;
    state.thoughtConfig.error = isError ? message : "";
    if (el.thoughtConfigResult) {
      el.thoughtConfigResult.textContent = message || "";
      el.thoughtConfigResult.classList.toggle("error", Boolean(isError));
    }
  }

  function renderOptions() {
    const backends = thoughtBackendMetadata(state.thoughtConfig);
    const currentBackend = normalizeBackendKey(el.thoughtConfigBackend.value || state.thoughtConfig.config?.backend || "");
    el.thoughtConfigBackend.innerHTML = "";
    for (const backend of backends) {
      const option = documentRef.createElement("option");
      option.value = backend.key;
      option.textContent = backend.label || backend.key || "auto";
      if (normalizeBackendKey(backend.key) === currentBackend) {
        option.selected = true;
      }
      el.thoughtConfigBackend.appendChild(option);
    }

    const selected = backends.find((backend) => normalizeBackendKey(backend.key) === currentBackend) ?? backends[0];
    el.thoughtConfigHint.textContent = selected?.model_presets_hint || "";
    const presets = Array.isArray(selected?.model_presets) ? selected.model_presets : [""];
    el.thoughtConfigModelPresets.innerHTML = "";
    for (const preset of presets) {
      const option = documentRef.createElement("option");
      option.value = preset;
      el.thoughtConfigModelPresets.appendChild(option);
    }
  }

  function applyToForm(payload) {
    const rawConfig = payload?.config || payload || null;
    const daemonDefaults = payload?.daemon_defaults ?? null;
    const ui = payload?.ui ?? null;
    const backend = normalizeBackendKey(rawConfig?.backend || "");
    const config = rawConfig
      ? {
          ...rawConfig,
          backend,
          model: normalizeThoughtModelForBackend(backend, rawConfig.model || ""),
        }
      : null;

    state.thoughtConfig.config = config;
    state.thoughtConfig.ui = ui;
    el.thoughtConfigEnabled.checked = Boolean(config?.enabled ?? true);
    el.thoughtConfigBackend.value = String(config?.backend || "");
    el.thoughtConfigModel.value = String(config?.model || "");
    renderOptions();
    const backendMetadata = selectedThoughtBackendMetadata(state.thoughtConfig);
    el.thoughtConfigSummary.textContent = backendMetadata
      ? `${backendMetadata.label || backendMetadata.key || "auto"} backend selected.`
      : "Thought config loaded.";
    const daemonBackend = normalizeBackendKey(daemonDefaults?.backend || "");
    el.thoughtConfigDaemon.textContent = daemonDefaults
      ? `daemon default: ${daemonBackend || "auto"} / ${daemonDefaults.model || "(empty)"}`
      : "daemon default: unavailable";
    syncSheetActionAvailability();
  }

  function draft() {
    if (!state.thoughtConfig.config) {
      return null;
    }
    return {
      ...state.thoughtConfig.config,
      enabled: Boolean(el.thoughtConfigEnabled.checked),
      backend: String(el.thoughtConfigBackend.value || "").trim(),
      model: String(el.thoughtConfigModel.value || "").trim(),
    };
  }

  async function refresh() {
    state.thoughtConfig.loading = true;
    try {
      const response = await apiFetch("/v1/thought-config");
      const payload = normalizeThoughtConfigResponse(await response.json());
      applyToForm(payload);
      setResult("Thought config loaded.");
    } catch (error) {
      setResult(`Failed to load thought config: ${error.message}`, true);
    } finally {
      state.thoughtConfig.loading = false;
      syncSheetActionAvailability();
    }
  }

  async function test() {
    const nextDraft = draft();
    if (!nextDraft) {
      return;
    }

    state.thoughtConfig.loading = true;
    setResult("Testing thought config...");
    try {
      const response = await apiFetch("/v1/thought-config/test", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(nextDraft),
      });
      const payload = normalizeThoughtConfigProbeResponse(await response.json());
      const message = payload?.message || "Thought config probe succeeded.";
      setResult(
        `${message}\n` +
          `ok: ${Boolean(payload?.ok)}\n` +
          `llm_calls: ${payload?.llm_calls ?? 0}\n` +
          (payload?.last_backend_error ? `backend error: ${payload.last_backend_error}` : ""),
      );
    } catch (error) {
      setResult(`Thought config test failed: ${error.message}`, true);
    } finally {
      state.thoughtConfig.loading = false;
      syncSheetActionAvailability();
    }
  }

  async function save() {
    const nextDraft = draft();
    if (!nextDraft) {
      return;
    }

    state.thoughtConfig.loading = true;
    setResult("Saving thought config...");
    try {
      const response = await apiFetch("/v1/thought-config", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(nextDraft),
      });
      await response.json();
      state.thoughtConfig.config = nextDraft;
      renderOptions();
      setResult("Thought config saved.");
      await refreshSessions();
    } catch (error) {
      setResult(`Thought config save failed: ${error.message}`, true);
    } finally {
      state.thoughtConfig.loading = false;
      syncSheetActionAvailability();
    }
  }

  async function handleFormSubmit(event) {
    event.preventDefault();
    await save();
  }

  function handleBackendChange() {
    el.thoughtConfigModel.value = normalizeThoughtModelForBackend(el.thoughtConfigBackend.value, el.thoughtConfigModel.value);
    renderOptions();
    syncSheetActionAvailability();
  }

  function handleOptionChange() {
    syncSheetActionAvailability();
  }

  async function handleTestButtonClick() {
    await test();
  }

  return {
    applyToForm,
    draft,
    handleBackendChange,
    handleFormSubmit,
    handleOptionChange,
    handleTestButtonClick,
    refresh,
    renderOptions,
    save,
    setResult,
    test,
  };
}
