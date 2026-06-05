export interface DirBrowserViewEntry {
  name?: string | null;
  full_path?: string | null;
  group?: string | null;
  groups?: readonly (string | null | undefined)[] | null;
  has_children?: boolean | null;
  is_running?: boolean | null;
  repo_dirty?: boolean | null;
  open_url?: string | null;
}

export interface DirBrowserViewState {
  groups?: readonly (string | null | undefined)[] | null;
  entries?: readonly DirBrowserViewEntry[] | null;
  path?: string | null;
  activeGroup?: string | null;
  selectedPaths?: Set<string> | readonly string[] | null;
  readOnly?: boolean;
  managed?: boolean;
  overlayLabel?: string | null;
  search?: string | null;
}

export interface DirBrowserViewIslandContainers {
  dirsGroups: HTMLElement;
  dirsList: HTMLElement;
}

export interface DirBrowserViewIslandHandle {
  containers: DirBrowserViewIslandContainers;
  groupsRoot: unknown;
  listRoot: unknown;
  render(view?: DirBrowserViewState): true;
  unmount(): void;
}

export const DIR_BROWSER_VIEW_ISLAND_IDS: Readonly<Record<string, string>>;
export const DIR_BROWSER_VIEW_ISLAND_KEYS: Readonly<Record<string, string>>;

export function createDirBrowserGroupChipElements(
  createElement: (...args: unknown[]) => unknown,
  view?: DirBrowserViewState,
): unknown[];
export function createDirBrowserListContents(
  createElement: (...args: unknown[]) => unknown,
  view?: DirBrowserViewState,
): unknown[];
export function DirBrowserGroups(props: { view?: DirBrowserViewState }): unknown;
export function DirBrowserList(props: { view?: DirBrowserViewState }): unknown;
export function resolveDirBrowserViewIslandContainers(options?: {
  documentRef?: Document;
  dirsGroups?: HTMLElement | { current?: HTMLElement | null } | null;
  dirsList?: HTMLElement | { current?: HTMLElement | null } | null;
}): DirBrowserViewIslandContainers;
export function assertStableDirBrowserViewIslandContainers(
  previous: DirBrowserViewIslandContainers,
  next: DirBrowserViewIslandContainers,
): DirBrowserViewIslandContainers;
export function mountDirBrowserViewIsland(options?: {
  documentRef?: Document;
  dirsGroups?: HTMLElement | { current?: HTMLElement | null } | null;
  dirsList?: HTMLElement | { current?: HTMLElement | null } | null;
  createRootImpl?: (root: HTMLElement) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
  flushSyncImpl?: (callback: () => void) => void;
}): DirBrowserViewIslandHandle;
