import type { BootPayload } from "./contracts.js";

export interface StableShellContainers {
  terminalStage: HTMLElement;
  terminalCanvas: HTMLCanvasElement;
  hudCanvas: HTMLCanvasElement;
  terminalFallback: HTMLPreElement;
  terminalA11yMirror: HTMLTextAreaElement;
  terminalAnnouncer: HTMLElement;
  trogdorSurface: HTMLElement;
}

export interface SwimmersRootShellProps {
  boot: Partial<BootPayload> | null | undefined;
}

export interface SwimmersRootShellHandle {
  root: HTMLElement;
  boot: BootPayload;
  containers: StableShellContainers;
  render(nextBoot?: Partial<BootPayload> | null | undefined): SwimmersRootShellHandle;
  unmount(): void;
}

export const SWIMMERS_REACT_ROOT_ID: "swimmers-react-root";
export const SWIMMERS_STABLE_CONTAINER_IDS: Readonly<Record<keyof StableShellContainers, string>>;

export function TerminalSurface(): unknown;
export function SwimmersRootShell(props: SwimmersRootShellProps): unknown;
export function resolveSwimmersReactRoot(documentRef?: Document): HTMLElement | null;
export function resolveStableShellContainers(documentRef?: Document): StableShellContainers;
export function assertStableShellContainerIdentity(
  previous: StableShellContainers,
  next: StableShellContainers,
): StableShellContainers;
export function mountSwimmersRootShell(options?: {
  documentRef?: Document;
  windowRef?: Window & { __SWIMMERS_BOOT__?: Partial<BootPayload> };
  root?: HTMLElement;
  boot?: Partial<BootPayload> | null | undefined;
  hydrateRootImpl?: (root: HTMLElement, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
}): SwimmersRootShellHandle;
