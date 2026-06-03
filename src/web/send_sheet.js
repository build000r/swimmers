function normalizedSendMode(sendMode) {
  return String(sendMode || "line") === "paste" ? "paste" : "line";
}

export function sendSheetSubmitPlan({
  readOnly = false,
  text = "",
  sendTarget = null,
  selectedSessionId = "",
  sendMode = "line",
} = {}) {
  const rawText = String(text ?? "");
  if (readOnly || !rawText.trim()) {
    return { type: "ignore" };
  }
  if (sendTarget?.type === "group") {
    return { type: "group", text: rawText, sessionIds: sendTarget.sessionIds };
  }
  const sessionId = sendTarget?.sessionId || selectedSessionId;
  return {
    type: normalizedSendMode(sendMode),
    text: rawText,
    sessionId,
    label: sendTarget?.label || sessionId,
  };
}

export function sendSheetSuccessStatus(plan, result) {
  if (plan?.type === "group") {
    const total = result?.total || plan.sessionIds.length;
    const skipped = result?.skipped || 0;
    const delivered = result?.delivered || 0;
    return {
      label: skipped > 0
        ? `Sent batch line to ${delivered} of ${total} agents.`
        : `Sent batch line to ${delivered} agents.`,
      muted: delivered === 0,
      ttlMs: skipped > 0 ? 3200 : 2400,
    };
  }
  return {
    label: plan?.type === "paste"
      ? `Pasted text to ${plan.label}.`
      : `Sent line to ${plan.label}.`,
    muted: false,
    ttlMs: 2200,
  };
}

export function sendSheetFailureStatus(error) {
  return {
    label: `Send failed: ${error.message}`,
    muted: true,
    ttlMs: 3200,
  };
}
