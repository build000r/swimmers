import { createTrogdorEventBindings } from "./trogdor_event_bindings.js";
import { createTrogdorSurfaceController } from "./trogdor_surface_controller.js";

export const TROGDOR_ATLAS_ISLAND_ID = "trogdor-surface";
export const TROGDOR_ATLAS_ISLAND_CLASS_NAME = "trogdor-surface hidden";
export const TROGDOR_ATLAS_ISLAND_ARIA_LABEL = "Trogdor repository atlas";

export const TROGDOR_ATLAS_ISLAND_PROPS = Object.freeze({
  className: TROGDOR_ATLAS_ISLAND_CLASS_NAME,
  id: TROGDOR_ATLAS_ISLAND_ID,
  "aria-label": TROGDOR_ATLAS_ISLAND_ARIA_LABEL,
});

export function createTrogdorAtlasIslandElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Trogdor atlas island requires a createElement function");
  }
  return createElement("section", TROGDOR_ATLAS_ISLAND_PROPS);
}

export function createTrogdorAtlasIsland(runtime = {}) {
  const el = runtime.el ?? runtime.elements ?? {};
  const surfaceController = createTrogdorSurfaceController({ ...runtime, el });
  let eventBindings = null;

  function trogdorEventBindings() {
    if (!eventBindings) {
      eventBindings = createTrogdorEventBindings({
        elements: el,
        ElementClass: runtime.ElementClass ?? globalThis.Element,
        handleSurfaceAction: runtime.handleSurfaceAction,
        openTrogdorAgentTerminal: runtime.openTrogdorAgentTerminal,
        openTrogdorAtlas: runtime.openTrogdorAtlas,
        updateHoveredTrogdorSurface: surfaceController.updateHoveredTrogdorSurface,
      });
    }
    return eventBindings;
  }

  function bindTrogdorEvents() {
    trogdorEventBindings().bindTrogdorEvents();
  }

  function handleTrogdorDomAction(button) {
    return trogdorEventBindings().handleTrogdorDomAction(button);
  }

  return {
    ...surfaceController,
    bindTrogdorEvents,
    handleTrogdorDomAction,
  };
}
