export interface NativeDesktopSheetIslandContainers {
  nativeSheet: HTMLElement;
  nativeSheetTitle: HTMLElement;
  nativeForm: HTMLFormElement;
  nativeStatusCopy: HTMLElement;
  nativeApp: HTMLSelectElement;
  nativeMode: HTMLSelectElement;
  nativeStatusResult: HTMLPreElement;
  nativeRefreshButton: HTMLButtonElement;
  nativeOpenButton: HTMLButtonElement;
  nativeCloseButton: HTMLButtonElement;
  nativeSaveButton: HTMLButtonElement;
}

export interface NativeDesktopSheetIslandHandle {
  containers: NativeDesktopSheetIslandContainers;
  reactRoot: unknown;
  render(): NativeDesktopSheetIslandHandle;
  unmount(): void;
}

export const NATIVE_DESKTOP_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const NATIVE_DESKTOP_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const NATIVE_DESKTOP_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const NATIVE_DESKTOP_DEFAULT_COPY: Readonly<Record<string, string>>;
export const NATIVE_DESKTOP_APP_OPTIONS: ReadonlyArray<Readonly<{ value: string; label: string }>>;
export const NATIVE_DESKTOP_MODE_OPTIONS: ReadonlyArray<Readonly<{ value: string; label: string }>>;

export function createNativeDesktopSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createNativeDesktopSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function NativeDesktopSheet(): unknown;
export function resolveNativeDesktopSheetIslandContainers(options?: {
  documentRef?: Document;
  nativeSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): NativeDesktopSheetIslandContainers;
export function assertStableNativeDesktopSheetIslandContainers(
  previous: NativeDesktopSheetIslandContainers,
  next: NativeDesktopSheetIslandContainers,
): NativeDesktopSheetIslandContainers;
export function mountNativeDesktopSheetIsland(options?: {
  documentRef?: Document;
  nativeSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): NativeDesktopSheetIslandHandle;
