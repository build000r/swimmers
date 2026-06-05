export interface WorkbenchWidgetViewItem {
  key?: string | null;
  title?: string | null;
  meta?: string | null;
  bodyHtml?: string | null;
  open?: boolean;
}

export interface WorkbenchWidgetsViewModel {
  statusText?: string | null;
  items?: readonly WorkbenchWidgetViewItem[] | null;
}

export interface WorkbenchWidgetsIslandContainers {
  terminalWorkbenchWidgets: HTMLElement;
}

export interface WorkbenchWidgetsIslandHandle {
  containers: WorkbenchWidgetsIslandContainers;
  reactRoot: unknown;
  render(model?: WorkbenchWidgetsViewModel): true;
  unmount(): void;
}

export const WORKBENCH_WIDGETS_ISLAND_IDS: Readonly<Record<string, string>>;

export function createWorkbenchWidgetsElements(
  createElement: (...args: unknown[]) => unknown,
  model?: WorkbenchWidgetsViewModel,
): unknown[];
export function WorkbenchWidgets(props: { model?: WorkbenchWidgetsViewModel }): unknown;
export function resolveWorkbenchWidgetsIslandContainers(options?: {
  documentRef?: Document;
  terminalWorkbenchWidgets?: HTMLElement | { current?: HTMLElement | null } | null;
}): WorkbenchWidgetsIslandContainers;
export function assertStableWorkbenchWidgetsIslandContainers(
  previous: WorkbenchWidgetsIslandContainers,
  next: WorkbenchWidgetsIslandContainers,
): WorkbenchWidgetsIslandContainers;
export function mountWorkbenchWidgetsIsland(options?: {
  documentRef?: Document;
  terminalWorkbenchWidgets?: HTMLElement | { current?: HTMLElement | null } | null;
  createRootImpl?: (root: HTMLElement) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
  flushSyncImpl?: (callback: () => void) => void;
}): WorkbenchWidgetsIslandHandle;
