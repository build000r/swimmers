const BASE = "/v1/thought-config";

export interface ThoughtConfig {
  enabled: boolean;
  model: string;
  cadence_hot_ms: number;
  cadence_warm_ms: number;
  cadence_cold_ms: number;
  agent_prompt: string;
  terminal_prompt: string;
}

interface ThoughtConfigPayload {
  enabled: boolean;
  model: string;
  cadence_hot_ms: number;
  cadence_warm_ms: number;
  cadence_cold_ms: number;
  agent_prompt?: string | null;
  terminal_prompt?: string | null;
}

interface ThoughtConfigEnvelope {
  config: ThoughtConfigPayload;
}

type ThoughtConfigWire = ThoughtConfigPayload | ThoughtConfigEnvelope;

function isThoughtConfigPayload(value: unknown): value is ThoughtConfigPayload {
  if (!value || typeof value !== "object") return false;
  const cfg = value as Partial<ThoughtConfigPayload>;
  return (
    typeof cfg.enabled === "boolean" &&
    typeof cfg.model === "string" &&
    typeof cfg.cadence_hot_ms === "number" &&
    typeof cfg.cadence_warm_ms === "number" &&
    typeof cfg.cadence_cold_ms === "number" &&
    (cfg.agent_prompt === undefined ||
      cfg.agent_prompt === null ||
      typeof cfg.agent_prompt === "string") &&
    (cfg.terminal_prompt === undefined ||
      cfg.terminal_prompt === null ||
      typeof cfg.terminal_prompt === "string")
  );
}

function unwrapConfig(payload: ThoughtConfigWire): ThoughtConfig {
  if (
    payload &&
    typeof payload === "object" &&
    "config" in payload &&
    isThoughtConfigPayload(payload.config)
  ) {
    return normalizeConfig(payload.config);
  }
  if (isThoughtConfigPayload(payload)) {
    return normalizeConfig(payload);
  }
  throw new Error("Invalid thought config payload");
}

function normalizeConfig(config: ThoughtConfigPayload): ThoughtConfig {
  return {
    enabled: config.enabled,
    model: config.model,
    cadence_hot_ms: config.cadence_hot_ms,
    cadence_warm_ms: config.cadence_warm_ms,
    cadence_cold_ms: config.cadence_cold_ms,
    agent_prompt: typeof config.agent_prompt === "string" ? config.agent_prompt : "",
    terminal_prompt:
      typeof config.terminal_prompt === "string" ? config.terminal_prompt : "",
  };
}

async function json<T>(res: Response): Promise<T | null> {
  const text = await res.text();
  let body: unknown = null;
  if (text.trim()) {
    try {
      body = JSON.parse(text) as unknown;
    } catch {
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      throw new Error("Invalid JSON response");
    }
  }

  if (!res.ok) {
    const errorBody =
      body && typeof body === "object"
        ? (body as { message?: unknown; code?: unknown })
        : null;
    const message =
      errorBody && typeof errorBody.message === "string"
        ? errorBody.message
        : errorBody && typeof errorBody.code === "string"
          ? errorBody.code
          : `HTTP ${res.status}`;
    throw new Error(message);
  }

  return body as T | null;
}

export async function fetchThoughtConfig(): Promise<ThoughtConfig> {
  const res = await fetch(BASE);
  const payload = await json<ThoughtConfigWire>(res);
  if (!payload) {
    throw new Error("Thought config response was empty.");
  }
  return unwrapConfig(payload);
}

export async function updateThoughtConfig(
  config: ThoughtConfig,
): Promise<ThoughtConfig> {
  const res = await fetch(BASE, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(config),
  });
  const payload = await json<ThoughtConfigWire>(res);
  if (!payload) {
    return config;
  }
  return unwrapConfig(payload);
}
