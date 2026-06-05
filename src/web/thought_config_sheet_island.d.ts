export interface ThoughtConfigSheetIslandContainers {
  thoughtConfigSheet: HTMLElement;
  thoughtConfigTitle: HTMLElement;
  thoughtConfigForm: HTMLFormElement;
  thoughtConfigEnabled: HTMLInputElement;
  thoughtConfigBackend: HTMLSelectElement;
  thoughtConfigModel: HTMLInputElement;
  thoughtConfigModelPresets: HTMLDataListElement;
  thoughtConfigHint: HTMLElement;
  thoughtConfigSummary: HTMLElement;
  thoughtConfigDaemon: HTMLElement;
  thoughtConfigResult: HTMLPreElement;
  thoughtConfigTestButton: HTMLButtonElement;
  thoughtConfigCloseButton: HTMLButtonElement;
  thoughtConfigSaveButton: HTMLButtonElement;
}

export interface ThoughtConfigSheetIslandHandle {
  containers: ThoughtConfigSheetIslandContainers;
  reactRoot: unknown;
  render(): ThoughtConfigSheetIslandHandle;
  unmount(): void;
}

export const THOUGHT_CONFIG_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const THOUGHT_CONFIG_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const THOUGHT_CONFIG_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const THOUGHT_CONFIG_MODEL_INPUT_PROPS: Readonly<Record<string, string>>;
export const THOUGHT_CONFIG_DEFAULT_COPY: Readonly<Record<string, string>>;

export function createThoughtConfigSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createThoughtConfigSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function ThoughtConfigSheet(): unknown;
export function resolveThoughtConfigSheetIslandContainers(options?: {
  documentRef?: Document;
  thoughtConfigSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): ThoughtConfigSheetIslandContainers;
export function assertStableThoughtConfigSheetIslandContainers(
  previous: ThoughtConfigSheetIslandContainers,
  next: ThoughtConfigSheetIslandContainers,
): ThoughtConfigSheetIslandContainers;
export function mountThoughtConfigSheetIsland(options?: {
  documentRef?: Document;
  thoughtConfigSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): ThoughtConfigSheetIslandHandle;
