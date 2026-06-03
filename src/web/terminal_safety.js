export const MAX_TERMINAL_PASTE_BYTES = 786432;

export function isLoopbackHostname(hostname) {
  const host = String(hostname || "").trim().toLowerCase().replace(/^\[|\]$/g, "");
  if (!host) {
    return false;
  }
  if (host === "localhost" || host.endsWith(".localhost") || host === "::1") {
    return true;
  }
  const ipv4 = host.split(".");
  return (
    ipv4.length === 4 &&
    ipv4[0] === "127" &&
    ipv4.every((part) => /^\d+$/.test(part) && Number(part) >= 0 && Number(part) <= 255)
  );
}

export function frankenTermLinkPolicy() {
  return {
    allowHttp: isLoopbackHostname(window.location?.hostname),
    allowHttps: true,
  };
}

export function safeAnchorHref(rawUrl) {
  try {
    const url = new URL(String(rawUrl || ""), window.location.href);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return "";
    }
    return url.toString();
  } catch (_) {
    return "";
  }
}

export function utf8ByteLength(text) {
  const value = String(text ?? "");
  if (typeof TextEncoder !== "undefined") {
    return new TextEncoder().encode(value).byteLength;
  }
  let count = 0;
  for (const char of value) {
    const code = char.codePointAt(0);
    if (code <= 0x7f) {
      count += 1;
    } else if (code <= 0x7ff) {
      count += 2;
    } else if (code <= 0xffff) {
      count += 3;
    } else {
      count += 4;
    }
  }
  return count;
}

export function terminalTextWithinPasteBudget(text) {
  return utf8ByteLength(text) <= MAX_TERMINAL_PASTE_BYTES;
}
