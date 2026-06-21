export interface CommandPaletteIslandItem {
  label?: string;
  meta?: string;
  disabled?: boolean;
  sessionId?: string;
  actionId?: string;
  action?: unknown;
}

export interface CommandPaletteIslandProps {
  items?: CommandPaletteIslandItem[];
  activeIndex?: number;
}

export interface CommandPaletteIslandContainers {
  paletteSheet: HTMLElement;
  paletteSearch: HTMLInputElement;
  paletteResults: HTMLElement;
  paletteCloseButton: HTMLButtonElement;
}

export interface CommandPaletteIslandHandle {
  containers: CommandPaletteIslandContainers;
  items: CommandPaletteIslandItem[];
  activeIndex: number;
  reactRoot: unknown;
  render(next?: CommandPaletteIslandProps): CommandPaletteIslandHandle;
  renderResults(next?: CommandPaletteIslandProps): true;
  unmount(): void;
}

export const COMMAND_PALETTE_ISLAND_IDS: Readonly<Record<string, string>>;
export const COMMAND_PALETTE_ISLAND_KEYS: Readonly<Record<string, string>>;
export const COMMAND_PALETTE_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const COMMAND_PALETTE_RESULTS_PROPS: Readonly<Record<string, string>>;

export function createCommandPaletteResultsElement(
  createElement: (...args: unknown[]) => unknown,
  props?: CommandPaletteIslandProps,
): unknown;
export function createCommandPaletteSheetContents(
  createElement: (...args: unknown[]) => unknown,
  props?: CommandPaletteIslandProps,
): unknown[];
export function createCommandPaletteSheetElement(
  createElement: (...args: unknown[]) => unknown,
  props?: CommandPaletteIslandProps,
): unknown;
export function CommandPaletteResults(props: CommandPaletteIslandProps): unknown;
export function CommandPaletteSheet(props: CommandPaletteIslandProps): unknown;
export function resolveCommandPaletteIslandHost(options?: {
  documentRef?: Document;
  paletteSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): HTMLElement;
export function resolveCommandPaletteIslandContainers(options?: {
  documentRef?: Document;
  paletteSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): CommandPaletteIslandContainers;
export function assertStableCommandPaletteIslandContainers(
  previous: CommandPaletteIslandContainers,
  next: CommandPaletteIslandContainers,
): CommandPaletteIslandContainers;
export function mountCommandPaletteIsland(options?: {
  documentRef?: Document;
  paletteSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  createRootImpl?: (root: HTMLElement) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
  flushSyncImpl?: (fn: () => void) => void;
  items?: CommandPaletteIslandItem[];
  activeIndex?: number;
}): CommandPaletteIslandHandle;
