function objectRecord(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : null;
}

function nonEmptyString(value) {
  const text = String(value ?? "").trim();
  return text || "";
}

export function splitNamespacedRemoteSessionId(session) {
  const sessionId = nonEmptyString(objectRecord(session)?.session_id);
  const separator = sessionId.indexOf("::");
  if (separator <= 0 || separator >= sessionId.length - 2) {
    return null;
  }
  return {
    targetId: sessionId.slice(0, separator),
    remoteSessionId: sessionId.slice(separator + 2),
  };
}

export function sessionEnvironment(session) {
  return objectRecord(objectRecord(session)?.environment) || {};
}

export function sessionUsesRemoteSnapshotFallback(session) {
  const environment = sessionEnvironment(session);
  const scope = nonEmptyString(environment.scope).toLowerCase();
  const targetId = nonEmptyString(environment.target_id);
  const targetKind = nonEmptyString(environment.target_kind).toLowerCase();
  const remoteSessionId = nonEmptyString(environment.remote_session_id);

  return Boolean(
    scope === "remote" ||
      remoteSessionId ||
      splitNamespacedRemoteSessionId(session) ||
      (targetId && targetId.toLowerCase() !== "local" && targetKind && targetKind !== "local"),
  );
}
