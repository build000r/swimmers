import type {
  BootstrapResponse,
  SessionListResponse,
  CreateSessionResponse,
  TerminalSnapshot,
  SessionPaneTailResponse,
  DirListResponse,
  SkillListResponse,
  SkillRegistryTool,
  SpawnTool,
  SessionDeleteMode,
} from "@/types";

const BASE = "/v1";

async function json<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.message ?? body.code ?? `HTTP ${res.status}`);
  }
  return res.json() as Promise<T>;
}

/** GET /v1/bootstrap - initial handshake + session list */
export async function bootstrap(): Promise<BootstrapResponse> {
  const res = await fetch(`${BASE}/bootstrap`);
  return json<BootstrapResponse>(res);
}

/** GET /v1/sessions - list all sessions with state */
export async function fetchSessions(): Promise<SessionListResponse> {
  const res = await fetch(`${BASE}/sessions`);
  return json<SessionListResponse>(res);
}

/** POST /v1/sessions - create a new tmux session */
export async function createSession(
  name?: string,
  cwd?: string,
  spawnTool?: SpawnTool,
): Promise<CreateSessionResponse> {
  const res = await fetch(`${BASE}/sessions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      name: name ?? null,
      cwd: cwd ?? null,
      spawn_tool: spawnTool ?? null,
    }),
  });
  return json<CreateSessionResponse>(res);
}

/** GET /v1/dirs - list subdirectories */
export async function listDirs(path?: string): Promise<DirListResponse> {
  const params = path ? `?path=${encodeURIComponent(path)}` : "";
  const res = await fetch(`${BASE}/dirs${params}`);
  return json<DirListResponse>(res);
}

/** GET /v1/skills?tool=claude|codex - list installed skills */
export async function listSkills(
  tool: SkillRegistryTool,
): Promise<SkillListResponse> {
  const res = await fetch(`${BASE}/skills?tool=${encodeURIComponent(tool)}`);
  return json<SkillListResponse>(res);
}

/** DELETE /v1/sessions/{id} - destroy a session */
export async function deleteSession(
  sessionId: string,
  mode: SessionDeleteMode = "detach_bridge",
): Promise<void> {
  const params = new URLSearchParams({ mode });
  const res = await fetch(
    `${BASE}/sessions/${encodeURIComponent(sessionId)}?${params.toString()}`,
    { method: "DELETE" },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.message ?? body.code ?? `HTTP ${res.status}`);
  }
}

/** POST /v1/sessions/{id}/attention/dismiss */
export async function dismissAttention(sessionId: string): Promise<void> {
  const res = await fetch(
    `${BASE}/sessions/${encodeURIComponent(sessionId)}/attention/dismiss`,
    { method: "POST" },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.message ?? body.code ?? `HTTP ${res.status}`);
  }
}

/** GET /v1/sessions/{id}/snapshot - terminal screen text + seq */
export async function fetchSnapshot(
  sessionId: string,
): Promise<TerminalSnapshot> {
  const res = await fetch(
    `${BASE}/sessions/${encodeURIComponent(sessionId)}/snapshot`,
  );
  return json<TerminalSnapshot>(res);
}

/** GET /v1/sessions/{id}/pane-tail - tmux captured pane text */
export async function fetchPaneTail(
  sessionId: string,
): Promise<SessionPaneTailResponse> {
  const res = await fetch(
    `${BASE}/sessions/${encodeURIComponent(sessionId)}/pane-tail`,
  );
  return json<SessionPaneTailResponse>(res);
}
