export function installEventListenerBinding(binding, runtime = {}) {
  const { document: documentRef = globalThis.document, elements = {}, handlers = {} } = runtime;
  const target = binding.target === "document" ? documentRef : elements[binding.target];
  if (binding.optionalTarget && !target) return;
  const handler = handlers[binding.handler];
  if (!handler) throw new Error(`Missing event listener handler: ${binding.handler}`);
  if (binding.optionalListener) {
    target.addEventListener?.(binding.eventType, handler, binding.options);
    return;
  }
  target.addEventListener(binding.eventType, handler, binding.options);
}

export function installEventListenerBindings(bindings, runtime = {}) {
  for (const binding of bindings) installEventListenerBinding(binding, runtime);
}

export function installTerminalStageCaptureBindings(bindings, runtime = {}) {
  const { captureSurfaceAction, elements = {} } = runtime;
  for (const binding of bindings) {
    elements.terminalStage.addEventListener(
      binding.eventType,
      (event) => captureSurfaceAction(event, binding.action),
      binding.options,
    );
  }
}

export function installTerminalStageResizeObserver(runtime = {}) {
  const {
    ResizeObserver: ResizeObserverCtor = globalThis.ResizeObserver,
    elements = {},
    queueMeasureAndResizeSurface,
  } = runtime;
  const resizeObserver = new ResizeObserverCtor(() => {
    queueMeasureAndResizeSurface(true, false);
  });
  resizeObserver.observe(elements.terminalStage);
  return resizeObserver;
}

export function bindAppEvents(runtime = {}) {
  runtime.bindTrogdorEvents();
  const bindingPlan = runtime.appEventListenerBindingPlan();
  installEventListenerBindings(bindingPlan.beforeTerminalStageCapture, runtime);
  installTerminalStageCaptureBindings(runtime.terminalStageCaptureBindings(), runtime);
  installEventListenerBindings(bindingPlan.afterTerminalStageCapture, runtime);
  return installTerminalStageResizeObserver(runtime);
}
