export type WorkspaceView = "overview" | "terminal";

export interface WorkspaceLayoutState {
  view: WorkspaceView;
  mainSessionId: string | null;
  bottomSessionId: string | null;
  splitRatio: number;
}

const PARAM_VIEW = "tt_view";
const PARAM_MAIN = "tt_main";
const PARAM_BOTTOM = "tt_bottom";
const PARAM_SPLIT = "tt_split";
const DEFAULT_SPLIT_RATIO = 0.6;

function normalizeSessionId(sessionId: string | null): string | null {
  const value = sessionId?.trim() ?? "";
  return value.length > 0 ? value : null;
}

function clampSplit(raw: number): number {
  if (Number.isNaN(raw) || !Number.isFinite(raw)) return DEFAULT_SPLIT_RATIO;
  return Math.max(0.2, Math.min(0.8, raw));
}

export function defaultWorkspaceLayout(): WorkspaceLayoutState {
  return {
    view: "overview",
    mainSessionId: null,
    bottomSessionId: null,
    splitRatio: DEFAULT_SPLIT_RATIO,
  };
}

export function parseWorkspaceLayoutFromUrl(url: URL): WorkspaceLayoutState {
  const view =
    url.searchParams.get(PARAM_VIEW) === "terminal" ? "terminal" : "overview";
  const splitRatio = clampSplit(
    Number.parseFloat(
      url.searchParams.get(PARAM_SPLIT) ?? String(DEFAULT_SPLIT_RATIO),
    ),
  );

  let mainSessionId = normalizeSessionId(url.searchParams.get(PARAM_MAIN));
  let bottomSessionId = normalizeSessionId(url.searchParams.get(PARAM_BOTTOM));

  if (mainSessionId && bottomSessionId && mainSessionId === bottomSessionId) {
    bottomSessionId = null;
  }

  if (!mainSessionId && bottomSessionId) {
    mainSessionId = bottomSessionId;
    bottomSessionId = null;
  }

  if (view === "overview" || (!mainSessionId && !bottomSessionId)) {
    return {
      view: "overview",
      mainSessionId: null,
      bottomSessionId: null,
      splitRatio,
    };
  }

  return {
    view: "terminal",
    mainSessionId,
    bottomSessionId,
    splitRatio,
  };
}

export function normalizeWorkspaceLayout(
  layout: WorkspaceLayoutState,
  availableSessionIds: Set<string>,
): WorkspaceLayoutState {
  let mainSessionId =
    layout.mainSessionId && availableSessionIds.has(layout.mainSessionId)
      ? layout.mainSessionId
      : null;
  let bottomSessionId =
    layout.bottomSessionId && availableSessionIds.has(layout.bottomSessionId)
      ? layout.bottomSessionId
      : null;

  if (mainSessionId && bottomSessionId && mainSessionId === bottomSessionId) {
    bottomSessionId = null;
  }

  if (!mainSessionId && bottomSessionId) {
    mainSessionId = bottomSessionId;
    bottomSessionId = null;
  }

  const splitRatio = clampSplit(layout.splitRatio);
  const view: WorkspaceView =
    layout.view === "terminal" && (mainSessionId || bottomSessionId)
      ? "terminal"
      : "overview";

  if (view === "overview") {
    return {
      view,
      mainSessionId: null,
      bottomSessionId: null,
      splitRatio,
    };
  }

  return {
    view,
    mainSessionId,
    bottomSessionId,
    splitRatio,
  };
}

export function applyWorkspaceLayoutToUrl(
  url: URL,
  layout: WorkspaceLayoutState,
): URL {
  const next = new URL(url.toString());
  let mainSessionId = normalizeSessionId(layout.mainSessionId);
  let bottomSessionId = normalizeSessionId(layout.bottomSessionId);
  const splitRatio = clampSplit(layout.splitRatio);

  if (mainSessionId && bottomSessionId && mainSessionId === bottomSessionId) {
    bottomSessionId = null;
  }

  if (!mainSessionId && bottomSessionId) {
    mainSessionId = bottomSessionId;
    bottomSessionId = null;
  }

  const normalizedView: WorkspaceView =
    layout.view === "terminal" && (mainSessionId || bottomSessionId)
      ? "terminal"
      : "overview";

  if (normalizedView === "overview") {
    next.searchParams.delete(PARAM_VIEW);
    next.searchParams.delete(PARAM_MAIN);
    next.searchParams.delete(PARAM_BOTTOM);
    next.searchParams.delete(PARAM_SPLIT);
    return next;
  }

  next.searchParams.set(PARAM_VIEW, "terminal");
  if (mainSessionId) {
    next.searchParams.set(PARAM_MAIN, mainSessionId);
  } else {
    next.searchParams.delete(PARAM_MAIN);
  }
  if (bottomSessionId) {
    next.searchParams.set(PARAM_BOTTOM, bottomSessionId);
  } else {
    next.searchParams.delete(PARAM_BOTTOM);
  }
  next.searchParams.set(PARAM_SPLIT, splitRatio.toFixed(2));
  return next;
}
