export interface SearchSheetIslandContainers {
  searchSheet: HTMLElement;
  searchForm: HTMLFormElement;
  terminalSearch: HTMLInputElement;
  searchPrevButton: HTMLButtonElement;
  searchNextButton: HTMLButtonElement;
  searchClearButton: HTMLButtonElement;
  searchCloseButton: HTMLButtonElement;
}

export interface SearchSheetIslandHandle {
  containers: SearchSheetIslandContainers;
  reactRoot: unknown;
  render(): SearchSheetIslandHandle;
  unmount(): void;
}

export const SEARCH_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const SEARCH_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const SEARCH_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const SEARCH_SHEET_INPUT_PROPS: Readonly<Record<string, string>>;

export function createSearchSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createSearchSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function SearchSheet(): unknown;
export function resolveSearchSheetIslandContainers(options?: {
  documentRef?: Document;
  searchSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): SearchSheetIslandContainers;
export function assertStableSearchSheetIslandContainers(
  previous: SearchSheetIslandContainers,
  next: SearchSheetIslandContainers,
): SearchSheetIslandContainers;
export function mountSearchSheetIsland(options?: {
  documentRef?: Document;
  searchSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): SearchSheetIslandHandle;
