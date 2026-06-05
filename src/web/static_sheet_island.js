export function mountHydratedStaticIsland({
  containers,
  hydrateRootImpl,
  renderElement,
  refreshContainers,
  assertStableContainers,
  root,
}) {
  if (typeof hydrateRootImpl !== "function") {
    throw new TypeError("Static sheet island requires a hydrateRoot function");
  }
  if (typeof renderElement !== "function") {
    throw new TypeError("Static sheet island requires a renderElement function");
  }
  if (typeof refreshContainers !== "function") {
    throw new TypeError("Static sheet island requires a refreshContainers function");
  }
  if (typeof assertStableContainers !== "function") {
    throw new TypeError("Static sheet island requires an assertStableContainers function");
  }
  const handle = {
    containers,
    reactRoot: null,
    render() {
      const previousContainers = handle.containers;
      handle.reactRoot?.render?.(renderElement());
      handle.containers = assertStableContainers(previousContainers, refreshContainers());
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(root, renderElement());
  return handle;
}
