import {
  trogdorDomActionZoneForDataset,
  trogdorSurfaceClickPlan,
  trogdorSurfaceFocusInPlan,
  trogdorSurfaceFocusOutPlan,
  trogdorSurfaceMouseleavePlan,
  trogdorSurfaceMouseoverPlan,
  trogdorSurfacePassthroughBindings,
  trogdorSurfacePointerDownPlan,
} from "./trogdor_logic.js";
import { shouldIgnoreSyntheticClick } from "./input_support.js";

const noop = () => {};

function trogdorEventTarget(event, ElementClass) {
  return ElementClass && event?.target instanceof ElementClass ? event.target : null;
}

function trogdorRelatedTarget(event, ElementClass) {
  return ElementClass && event?.relatedTarget instanceof ElementClass ? event.relatedTarget : null;
}

export function createTrogdorEventBindings(runtime = {}) {
  const {
    elements = {},
    ElementClass = globalThis.Element,
    handleSurfaceAction = noop,
    openTrogdorAgentTerminal = noop,
    openTrogdorAtlas = noop,
    updateHoveredTrogdorSurface = noop,
    now = () => Date.now(),
    surfaceClickSuppressMs = 450,
  } = runtime;

  // After a pointerdown opens an agent, ignore the synthetic mouse/touch click
  // that follows on the same element so the terminal does not open twice.
  // Keyboard activation fires click with no preceding pointerdown, so it is
  // never suppressed.
  let clickSuppressUntil = 0;

  async function handleTrogdorDomAction(button) {
    if (!button || button.disabled) {
      return;
    }
    await handleSurfaceAction(trogdorDomActionZoneForDataset(button.dataset));
  }

  function handleTrogdorLauncherClick(event) {
    event.preventDefault();
    openTrogdorAtlas();
  }

  function handleTrogdorSurfacePointerDown(event) {
    const plan = trogdorSurfacePointerDownPlan(trogdorEventTarget(event, ElementClass));
    if (plan.type !== "open_agent_terminal") {
      return;
    }
    if (plan.preventDefault) event.preventDefault();
    if (plan.stopPropagation) event.stopPropagation();
    void openTrogdorAgentTerminal(plan.sessionId);
    clickSuppressUntil = now() + surfaceClickSuppressMs;
  }

  function handleTrogdorSurfacePassthrough(event) {
    event.stopPropagation();
  }

  function installTrogdorSurfacePassthroughBindings() {
    for (const binding of trogdorSurfacePassthroughBindings()) {
      elements.trogdorSurface.addEventListener(
        binding.eventName,
        handleTrogdorSurfacePassthrough,
        binding.options,
      );
    }
  }

  function handleTrogdorSurfaceClick(event) {
    const plan = trogdorSurfaceClickPlan(trogdorEventTarget(event, ElementClass));
    if (plan.preventDefault) event.preventDefault();
    if (plan.stopPropagation) event.stopPropagation();
    if (plan.type === "dom_action") {
      void handleTrogdorDomAction(plan.button);
      return;
    }
    if (plan.type === "surface_action") {
      // The preceding pointerdown already opened this agent; skip the synthetic
      // click. A keyboard-driven click has no prior pointerdown and proceeds.
      if (shouldIgnoreSyntheticClick(now(), clickSuppressUntil)) {
        return;
      }
      void handleSurfaceAction(plan.zone);
    }
  }

  function handleTrogdorSurfaceMouseover(event) {
    const plan = trogdorSurfaceMouseoverPlan(trogdorEventTarget(event, ElementClass));
    if (plan.type === "hover") updateHoveredTrogdorSurface(plan.hover);
  }

  function handleTrogdorSurfaceMouseleave() {
    updateHoveredTrogdorSurface(trogdorSurfaceMouseleavePlan().hover);
  }

  function handleTrogdorSurfaceFocusIn(event) {
    const plan = trogdorSurfaceFocusInPlan(trogdorEventTarget(event, ElementClass));
    if (plan.type === "hover") updateHoveredTrogdorSurface(plan.hover);
  }

  function handleTrogdorSurfaceFocusOut(event) {
    const next = trogdorRelatedTarget(event, ElementClass);
    const plan = trogdorSurfaceFocusOutPlan({
      relatedTargetInsideSurface: Boolean(next && elements.trogdorSurface.contains(next)),
    });
    if (plan.type === "clear_hover") updateHoveredTrogdorSurface(plan.hover);
  }

  function bindTrogdorEvents() {
    if (elements.trogdorLauncher) {
      elements.trogdorLauncher.addEventListener("click", handleTrogdorLauncherClick);
    }

    if (!elements.trogdorSurface) {
      return;
    }

    elements.trogdorSurface.addEventListener("pointerdown", handleTrogdorSurfacePointerDown);
    installTrogdorSurfacePassthroughBindings();
    elements.trogdorSurface.addEventListener("click", handleTrogdorSurfaceClick);
    elements.trogdorSurface.addEventListener("mouseover", handleTrogdorSurfaceMouseover);
    elements.trogdorSurface.addEventListener("mouseleave", handleTrogdorSurfaceMouseleave);
    elements.trogdorSurface.addEventListener("focusin", handleTrogdorSurfaceFocusIn);
    elements.trogdorSurface.addEventListener("focusout", handleTrogdorSurfaceFocusOut);
  }

  return {
    bindTrogdorEvents,
    handleTrogdorDomAction,
    handleTrogdorLauncherClick,
    handleTrogdorSurfaceClick,
    handleTrogdorSurfaceFocusIn,
    handleTrogdorSurfaceFocusOut,
    handleTrogdorSurfaceMouseleave,
    handleTrogdorSurfaceMouseover,
    handleTrogdorSurfacePassthrough,
    handleTrogdorSurfacePointerDown,
  };
}

export function bindTrogdorEvents(runtime = {}) {
  createTrogdorEventBindings(runtime).bindTrogdorEvents();
}
