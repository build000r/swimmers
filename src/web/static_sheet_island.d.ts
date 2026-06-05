export interface StaticSheetIslandHandle<TContainers> {
  containers: TContainers;
  reactRoot: unknown;
  render(): StaticSheetIslandHandle<TContainers>;
  unmount(): void;
}

export function mountHydratedStaticIsland<TContainers>(options: {
  containers: TContainers;
  hydrateRootImpl: (root: unknown, element: unknown) => {
    render?: (element: unknown) => void;
    unmount?: () => void;
  };
  renderElement: () => unknown;
  refreshContainers: () => TContainers;
  assertStableContainers: (previous: TContainers, next: TContainers) => TContainers;
  root: unknown;
}): StaticSheetIslandHandle<TContainers>;
