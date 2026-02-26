import { useCallback, useEffect, useState } from "preact/hooks";
import {
  fetchThoughtConfig,
  updateThoughtConfig,
  type ThoughtConfig,
} from "@/services/thought-config";
import "./ThoughtConfigPanel.css";

interface ThoughtConfigPanelProps {
  open: boolean;
  observer?: boolean;
  onClose: () => void;
}

interface ThoughtConfigForm {
  enabled: boolean;
  model: string;
  cadence_hot_ms: string;
  cadence_warm_ms: string;
  cadence_cold_ms: string;
  agent_prompt: string;
  terminal_prompt: string;
}

type TextField =
  | "model"
  | "cadence_hot_ms"
  | "cadence_warm_ms"
  | "cadence_cold_ms"
  | "agent_prompt"
  | "terminal_prompt";

function toForm(config: ThoughtConfig): ThoughtConfigForm {
  return {
    enabled: config.enabled,
    model: config.model,
    cadence_hot_ms: String(config.cadence_hot_ms),
    cadence_warm_ms: String(config.cadence_warm_ms),
    cadence_cold_ms: String(config.cadence_cold_ms),
    agent_prompt: config.agent_prompt,
    terminal_prompt: config.terminal_prompt,
  };
}

function parseCadence(raw: string): number | null {
  const value = Number(raw);
  if (!Number.isFinite(value) || !Number.isInteger(value) || value <= 0) {
    return null;
  }
  return value;
}

function validateForm(
  form: ThoughtConfigForm,
): { config: ThoughtConfig | null; error: string | null } {
  const model = form.model.trim();

  const hot = parseCadence(form.cadence_hot_ms);
  if (hot === null) {
    return {
      config: null,
      error: "Hot cadence must be a positive integer in milliseconds.",
    };
  }

  const warm = parseCadence(form.cadence_warm_ms);
  if (warm === null) {
    return {
      config: null,
      error: "Warm cadence must be a positive integer in milliseconds.",
    };
  }

  const cold = parseCadence(form.cadence_cold_ms);
  if (cold === null) {
    return {
      config: null,
      error: "Cold cadence must be a positive integer in milliseconds.",
    };
  }

  if (warm < hot) {
    return {
      config: null,
      error: "Warm cadence must be greater than or equal to hot cadence.",
    };
  }

  if (cold < warm) {
    return {
      config: null,
      error: "Cold cadence must be greater than or equal to warm cadence.",
    };
  }

  return {
    error: null,
    config: {
      enabled: form.enabled,
      model,
      cadence_hot_ms: hot,
      cadence_warm_ms: warm,
      cadence_cold_ms: cold,
      agent_prompt: form.agent_prompt,
      terminal_prompt: form.terminal_prompt,
    },
  };
}

export function ThoughtConfigPanel({
  open,
  observer = false,
  onClose,
}: ThoughtConfigPanelProps) {
  const [form, setForm] = useState<ThoughtConfigForm | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveNotice, setSaveNotice] = useState<string | null>(null);

  const loadConfig = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    setValidationError(null);
    setSaveError(null);
    setSaveNotice(null);
    try {
      const config = await fetchThoughtConfig();
      setForm(toForm(config));
    } catch (error) {
      setForm(null);
      setLoadError(
        error instanceof Error
          ? error.message
          : "Failed to load thought configuration.",
      );
    } finally {
      setLoading(false);
    }
  }, []);

  const updateTextField = useCallback((field: TextField, value: string) => {
    setForm((prev) => (prev ? { ...prev, [field]: value } : prev));
    setValidationError(null);
    setSaveError(null);
    setSaveNotice(null);
  }, []);

  const updateEnabled = useCallback((enabled: boolean) => {
    setForm((prev) => (prev ? { ...prev, enabled } : prev));
    setValidationError(null);
    setSaveError(null);
    setSaveNotice(null);
  }, []);

  const handleSave = useCallback(
    async (event: Event) => {
      event.preventDefault();
      event.stopPropagation();
      if (observer || !form || saving) return;

      setValidationError(null);
      setSaveError(null);
      setSaveNotice(null);

      const validated = validateForm(form);
      if (!validated.config) {
        setValidationError(validated.error ?? "Invalid thought configuration.");
        return;
      }

      setSaving(true);
      try {
        const updated = await updateThoughtConfig(validated.config);
        setForm(toForm(updated));
        setSaveNotice("Thought configuration saved.");
      } catch (error) {
        setSaveError(
          error instanceof Error
            ? error.message
            : "Failed to save thought configuration.",
        );
      } finally {
        setSaving(false);
      }
    },
    [form, observer, saving],
  );

  useEffect(() => {
    if (!open) return;
    void loadConfig();
  }, [open, loadConfig]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const inputsDisabled = observer || loading || saving || !form;

  return (
    <div
      class="thought-config-overlay"
      onClick={(event: MouseEvent) => {
        event.stopPropagation();
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
      onTouchEnd={(event: TouchEvent) => {
        event.stopPropagation();
      }}
    >
      <div
        class="thought-config-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Thought configuration"
        onClick={(event: MouseEvent) => event.stopPropagation()}
        onTouchEnd={(event: TouchEvent) => event.stopPropagation()}
      >
        <div class="thought-config-header">
          <div>
            <h2>Thought Config</h2>
            <p>Tune thought generation cadence and prompts.</p>
          </div>
          <button
            type="button"
            class="thought-config-close"
            onClick={(event: MouseEvent) => {
              event.stopPropagation();
              onClose();
            }}
          >
            Close
          </button>
        </div>

        <form class="thought-config-form" onSubmit={handleSave}>
          {observer && (
            <p class="thought-config-note" role="status">
              Observer mode is read-only.
            </p>
          )}

          {loading && <p class="thought-config-status">Loading configuration...</p>}

          {loadError && (
            <div class="thought-config-error" role="alert">
              <span>{loadError}</span>
              <button
                type="button"
                class="thought-config-inline-btn"
                onClick={() => void loadConfig()}
              >
                Retry
              </button>
            </div>
          )}

          {form && (
            <>
              <label class="thought-config-checkbox">
                <input
                  type="checkbox"
                  checked={form.enabled}
                  disabled={inputsDisabled}
                  onChange={(event: Event) =>
                    updateEnabled(
                      (event.currentTarget as HTMLInputElement).checked,
                    )}
                />
                <span>Enabled</span>
              </label>

              <label class="thought-config-field">
                <span>Model</span>
                <input
                  type="text"
                  value={form.model}
                  disabled={inputsDisabled}
                  onInput={(event: Event) =>
                    updateTextField(
                      "model",
                      (event.currentTarget as HTMLInputElement).value,
                    )}
                />
              </label>

              <div class="thought-config-cadence-grid">
                <label class="thought-config-field">
                  <span>Cadence hot (ms)</span>
                  <input
                    type="number"
                    min="1"
                    step="1000"
                    value={form.cadence_hot_ms}
                    disabled={inputsDisabled}
                    onInput={(event: Event) =>
                      updateTextField(
                        "cadence_hot_ms",
                        (event.currentTarget as HTMLInputElement).value,
                      )}
                  />
                </label>

                <label class="thought-config-field">
                  <span>Cadence warm (ms)</span>
                  <input
                    type="number"
                    min="1"
                    step="1000"
                    value={form.cadence_warm_ms}
                    disabled={inputsDisabled}
                    onInput={(event: Event) =>
                      updateTextField(
                        "cadence_warm_ms",
                        (event.currentTarget as HTMLInputElement).value,
                      )}
                  />
                </label>

                <label class="thought-config-field">
                  <span>Cadence cold (ms)</span>
                  <input
                    type="number"
                    min="1"
                    step="1000"
                    value={form.cadence_cold_ms}
                    disabled={inputsDisabled}
                    onInput={(event: Event) =>
                      updateTextField(
                        "cadence_cold_ms",
                        (event.currentTarget as HTMLInputElement).value,
                      )}
                  />
                </label>
              </div>

              <label class="thought-config-field">
                <span>Agent prompt</span>
                <textarea
                  rows={4}
                  value={form.agent_prompt}
                  disabled={inputsDisabled}
                  onInput={(event: Event) =>
                    updateTextField(
                      "agent_prompt",
                      (event.currentTarget as HTMLTextAreaElement).value,
                    )}
                />
              </label>

              <label class="thought-config-field">
                <span>Terminal prompt</span>
                <textarea
                  rows={4}
                  value={form.terminal_prompt}
                  disabled={inputsDisabled}
                  onInput={(event: Event) =>
                    updateTextField(
                      "terminal_prompt",
                      (event.currentTarget as HTMLTextAreaElement).value,
                    )}
                />
              </label>
            </>
          )}

          {validationError && (
            <p class="thought-config-error" role="alert">
              {validationError}
            </p>
          )}

          {saveError && (
            <p class="thought-config-error" role="alert">
              {saveError}
            </p>
          )}

          {saveNotice && (
            <p class="thought-config-success" role="status">
              {saveNotice}
            </p>
          )}

          <div class="thought-config-actions">
            <button
              type="button"
              class="thought-config-btn secondary"
              onClick={onClose}
            >
              Close
            </button>
            <button
              type="submit"
              class="thought-config-btn primary"
              disabled={observer || loading || saving || !form}
            >
              {observer ? "Read only" : saving ? "Saving..." : "Save"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
