export interface AuthSheetIslandContainers {
  authSheet: HTMLElement;
  authSheetTitle: HTMLElement;
  tokenInput: HTMLInputElement;
  clearTokenButton: HTMLButtonElement;
  authCloseButton: HTMLButtonElement;
  saveTokenButton: HTMLButtonElement;
}

export interface AuthSheetIslandHandle {
  containers: AuthSheetIslandContainers;
  reactRoot: unknown;
  render(): AuthSheetIslandHandle;
  unmount(): void;
}

export const AUTH_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const AUTH_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const AUTH_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const AUTH_SHEET_TOKEN_INPUT_PROPS: Readonly<Record<string, string>>;
export const AUTH_SHEET_COPY: string;

export function createAuthSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createAuthSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function AuthSheet(): unknown;
export function resolveAuthSheetIslandContainers(options?: {
  documentRef?: Document;
  authSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): AuthSheetIslandContainers;
export function assertStableAuthSheetIslandContainers(
  previous: AuthSheetIslandContainers,
  next: AuthSheetIslandContainers,
): AuthSheetIslandContainers;
export function mountAuthSheetIsland(options?: {
  documentRef?: Document;
  authSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): AuthSheetIslandHandle;
