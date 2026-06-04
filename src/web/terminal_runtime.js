export function surfaceSupports(surface, methodName) {
  return Boolean(surface && typeof surface[methodName] === "function");
}

export function assertFrankenTermModule(mod) {
  if (!mod || typeof mod.default !== "function") {
    throw new Error("FrankenTerm module is missing its wasm initializer");
  }
  if (typeof mod.FrankenTermWeb !== "function") {
    throw new Error("FrankenTerm module is missing FrankenTermWeb");
  }
  return mod;
}

export function validateFrankenTermSurface(surface, requiredMethods, label = "FrankenTerm surface") {
  const missing = requiredMethods.filter((methodName) => !surfaceSupports(surface, methodName));
  if (missing.length) {
    throw new Error(`${label} missing methods: ${missing.join(", ")}`);
  }
  return surface;
}

export function frankenTermAssetSummary(info) {
  if (!info || typeof info !== "object") {
    return "";
  }

  const pieces = [];
  for (const key of ["js", "wasm", "font"]) {
    const item = info[key];
    if (!item) {
      continue;
    }
    const checksum = item.checksum ? ` ${item.checksum}` : "";
    const size = Number.isFinite(item.size_bytes) ? ` ${item.size_bytes}b` : "";
    pieces.push(`${key}${checksum}${size}`);
  }
  return pieces.join("; ");
}

export function surfaceBusy(surfaceState) {
  return surfaceState.surfaceInitInProgress > 0 || surfaceState.surfaceOperationDepth > 0;
}

export function frankenTermErrorMessage(error) {
  return error?.message || String(error || "");
}

export function isFrankenTermReentryError(error) {
  return /recursive use of an object/i.test(frankenTermErrorMessage(error));
}

export function withSurfaceOperation(surfaceState, label, callback) {
  if (surfaceBusy(surfaceState)) {
    return { deferred: true };
  }

  surfaceState.surfaceOperationDepth += 1;
  try {
    return { deferred: false, value: callback() };
  } catch (error) {
    if (isFrankenTermReentryError(error)) {
      surfaceState.lastRendererDiagnosticError = `${label}: ${frankenTermErrorMessage(error)}`;
      return { deferred: true, error };
    }
    throw error;
  } finally {
    surfaceState.surfaceOperationDepth -= 1;
  }
}

export function canvasHasVisiblePixels(canvas, documentRef) {
  if (!canvas || !canvas.width || !canvas.height) {
    return false;
  }

  const sample = documentRef.createElement("canvas");
  sample.width = Math.min(180, canvas.width);
  sample.height = Math.min(120, canvas.height);
  if (!sample.width || !sample.height) {
    return false;
  }

  const context = sample.getContext("2d", { willReadFrequently: true });
  if (!context) {
    return false;
  }

  try {
    context.drawImage(canvas, 0, 0, sample.width, sample.height);
    const pixels = context.getImageData(0, 0, sample.width, sample.height).data;
    for (let index = 0; index < pixels.length; index += 4) {
      const alpha = pixels[index + 3];
      const red = pixels[index];
      const green = pixels[index + 1];
      const blue = pixels[index + 2];
      if (alpha > 0 && (red > 32 || green > 32 || blue > 32)) {
        return true;
      }
    }
  } catch (_) {
    return false;
  }

  return false;
}
