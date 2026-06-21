export function elementFromRef(ref) {
  return ref?.current ?? ref;
}

// Tracked-node identity drift is normally recoverable (React would just re-key
// the subtree), so throwing in production turns a cosmetic markup hiccup into a
// hard crash of the live terminal surface. We therefore throw only in dev, and
// downgrade to console.error + telemetry in production so the surface survives.
let identityDriftReporter = null;

export function setIdentityDriftReporter(reporter) {
  identityDriftReporter = typeof reporter === "function" ? reporter : null;
  return identityDriftReporter;
}

function isDevEnvironment() {
  try {
    // Vite injects import.meta.env.DEV; absent (e.g. node tests) means "not dev".
    return Boolean(import.meta?.env?.DEV);
  } catch {
    return false;
  }
}

export function reportIdentityDrift(message, details = {}) {
  if (typeof console !== "undefined" && typeof console.error === "function") {
    console.error(`[swimmers-web] ${message}`, details);
  }
  if (identityDriftReporter) {
    try {
      identityDriftReporter({ message, ...details });
    } catch {
      // A broken telemetry sink must never escalate into a surface crash.
    }
  }
}

// Default contract is unchanged: drift throws. Callers guarding a surface whose
// crash would be worse than the drift itself (e.g. the live terminal shell) pass
// throwOnDrift:false to downgrade to console.error + telemetry, and dev still
// throws so the mismatch is caught before it ships.
export function assertStableIdentity(previous, next, {
  keys = Object.keys(previous || {}),
  label = "React island",
  noun = "container",
  throwOnDrift = true,
} = {}) {
  const shouldThrow = throwOnDrift || isDevEnvironment();
  for (const key of keys) {
    if (previous?.[key] !== next?.[key]) {
      const message = `${label} replaced stable ${noun} ${key}`;
      if (shouldThrow) {
        throw new Error(message);
      }
      reportIdentityDrift(message, { label, noun, key });
      // Keep going so every drifted key is surfaced, then return the latest
      // containers so the caller stays pointed at the live nodes.
    }
  }
  return next;
}
