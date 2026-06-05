export interface SendSheetIslandContainers {
  sendSheet: HTMLElement;
  sendSheetTitle: HTMLElement;
  sendForm: HTMLFormElement;
  sendMode: HTMLSelectElement;
  sendInput: HTMLTextAreaElement;
  sendHistory: HTMLElement;
  sendHint: HTMLElement;
  sendCloseButton: HTMLButtonElement;
  sendSubmitButton: HTMLButtonElement;
}

export interface SendSheetIslandHandle {
  containers: SendSheetIslandContainers;
  reactRoot: unknown;
  render(): SendSheetIslandHandle;
  unmount(): void;
}

export const SEND_SHEET_ISLAND_IDS: Readonly<Record<string, string>>;
export const SEND_SHEET_ISLAND_KEYS: Readonly<Record<string, string>>;
export const SEND_SHEET_ISLAND_HOST_PROPS: Readonly<Record<string, string>>;
export const SEND_SHEET_INPUT_PROPS: Readonly<{
  id: string;
  rows: number;
  placeholder: string;
}>;
export const SEND_SHEET_HISTORY_PROPS: Readonly<Record<string, string>>;

export function createSendSheetContents(createElement: (...args: unknown[]) => unknown): unknown[];
export function createSendSheetElement(createElement: (...args: unknown[]) => unknown): unknown;
export function SendSheet(): unknown;
export function resolveSendSheetIslandContainers(options?: {
  documentRef?: Document;
  sendSheet?: HTMLElement | { current?: HTMLElement | null } | null;
}): SendSheetIslandContainers;
export function assertStableSendSheetIslandContainers(
  previous: SendSheetIslandContainers,
  next: SendSheetIslandContainers,
): SendSheetIslandContainers;
export function mountSendSheetIsland(options?: {
  documentRef?: Document;
  sendSheet?: HTMLElement | { current?: HTMLElement | null } | null;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): SendSheetIslandHandle;
