export interface MermaidSheetIslandContainers {
  mermaidSheet: HTMLElement;
  mermaidSheetTitle: HTMLElement;
  mermaidSummary: HTMLElement;
  mermaidPreview: HTMLElement;
  mermaidSource: HTMLPreElement;
  mermaidPlanTabs: HTMLElement;
  mermaidPlanContent: HTMLPreElement;
  mermaidRefreshButton: HTMLButtonElement;
  mermaidOpenButton: HTMLButtonElement;
  mermaidCloseButton: HTMLButtonElement;
}

export interface MermaidSheetIslandHandle {
  containers: MermaidSheetIslandContainers;
  reactRoot: unknown;
  render(): MermaidSheetIslandHandle;
  unmount(): void;
}

export const MERMAID_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const MERMAID_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const MERMAID_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const MERMAID_SHEET_DEFAULT_COPY: Readonly<Record<string, string>>;

export function createMermaidSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createMermaidSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function MermaidSheet(): unknown;
export function resolveMermaidSheetIslandContainers(options?: {
  documentRef?: Document;
  mermaidSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): MermaidSheetIslandContainers;
export function assertStableMermaidSheetIslandContainers(
  previous: MermaidSheetIslandContainers,
  next: MermaidSheetIslandContainers,
): MermaidSheetIslandContainers;
export function mountMermaidSheetIsland(options?: {
  documentRef?: Document;
  mermaidSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): MermaidSheetIslandHandle;
