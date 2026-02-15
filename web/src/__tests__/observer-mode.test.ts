import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { useObserverMode } from "@/hooks/useObserverMode";
import { renderHook } from "@testing-library/preact";

describe("useObserverMode", () => {
  const originalLocation = window.location;

  function setUrlParam(param: string) {
    // happy-dom allows direct assignment
    Object.defineProperty(window, "location", {
      value: {
        ...originalLocation,
        search: param,
        href: `http://localhost${param}`,
      },
      writable: true,
      configurable: true,
    });
  }

  afterEach(() => {
    Object.defineProperty(window, "location", {
      value: originalLocation,
      writable: true,
      configurable: true,
    });
  });

  it("returns false when no observer indicators are present", () => {
    setUrlParam("");
    const { result } = renderHook(() => useObserverMode());
    expect(result.current.isObserver).toBe(false);
  });

  it("returns true when URL has ?mode=observer", () => {
    setUrlParam("?mode=observer");
    const { result } = renderHook(() => useObserverMode());
    expect(result.current.isObserver).toBe(true);
  });

  it("returns false when URL has ?mode=operator", () => {
    setUrlParam("?mode=operator");
    const { result } = renderHook(() => useObserverMode());
    expect(result.current.isObserver).toBe(false);
  });

  it("returns true when authMode is 'observer'", () => {
    setUrlParam("");
    const { result } = renderHook(() => useObserverMode("observer"));
    expect(result.current.isObserver).toBe(true);
  });

  it("returns false when authMode is 'operator'", () => {
    setUrlParam("");
    const { result } = renderHook(() => useObserverMode("operator"));
    expect(result.current.isObserver).toBe(false);
  });

  it("URL param takes precedence (observer URL overrides operator auth)", () => {
    setUrlParam("?mode=observer");
    const { result } = renderHook(() => useObserverMode("operator"));
    expect(result.current.isObserver).toBe(true);
  });
});
