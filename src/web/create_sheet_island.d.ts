export interface CreateSheetIslandContainers {
  createSheet: HTMLElement;
  createSheetTitle: HTMLElement;
  createCloseButton: HTMLButtonElement;
  dirsSearch: HTMLInputElement;
  dirsManagedOnly: HTMLInputElement;
  createBatchVisible: HTMLButtonElement;
  dirsGroups: HTMLElement;
  dirsPath: HTMLInputElement;
  dirsUpButton: HTMLButtonElement;
  dirsLoadButton: HTMLButtonElement;
  dirsSpawnHere: HTMLButtonElement;
  dirsList: HTMLElement;
  createForm: HTMLFormElement;
  createCwd: HTMLInputElement;
  createTool: HTMLSelectElement;
  createLaunchTarget: HTMLSelectElement;
  createRequest: HTMLTextAreaElement;
  dirsSummary: HTMLElement;
  createBatchBar: HTMLElement;
  createBatchCount: HTMLElement;
  createBatchTool: HTMLElement;
  createBatchPreview: HTMLElement;
  createBatchClear: HTMLButtonElement;
  createBatchSubmit: HTMLButtonElement;
  createButton: HTMLButtonElement;
}

export interface CreateSheetIslandHandle {
  containers: CreateSheetIslandContainers;
  reactRoot: unknown;
  render(): CreateSheetIslandHandle;
  unmount(): void;
}

export const CREATE_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const CREATE_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const CREATE_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const CREATE_SHEET_SEARCH_INPUT_PROPS: Readonly<Record<string, string>>;
export const CREATE_SHEET_PATH_INPUT_PROPS: Readonly<Record<string, string>>;
export const CREATE_SHEET_CWD_INPUT_PROPS: Readonly<Record<string, string>>;
export const CREATE_SHEET_REQUEST_PROPS: Readonly<{
  id: string;
  rows: number;
  placeholder: string;
}>;
export const CREATE_SHEET_DIR_LIST_PROPS: Readonly<Record<string, string>>;
export const CREATE_SHEET_DEFAULT_COPY: Readonly<Record<string, string>>;
export const CREATE_SHEET_TOOL_OPTIONS: readonly Readonly<{
  value: string;
  label: string;
}>[];

export function createCreateSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createCreateSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function CreateSheet(): unknown;
export function resolveCreateSheetIslandContainers(options?: {
  documentRef?: Document;
  createSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): CreateSheetIslandContainers;
export function assertStableCreateSheetIslandContainers(
  previous: CreateSheetIslandContainers,
  next: CreateSheetIslandContainers,
): CreateSheetIslandContainers;
export function mountCreateSheetIsland(options?: {
  documentRef?: Document;
  createSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): CreateSheetIslandHandle;
