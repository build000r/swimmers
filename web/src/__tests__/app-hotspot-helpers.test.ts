import { describe, expect, it } from "vitest";
import type { NativeDesktopStatus } from "@/types";
import { makeSession } from "./helpers/fixtures";
import { deriveAppViewFlags, resolveOverviewTapIntent } from "@/app";

const NATIVE_SUPPORTED: NativeDesktopStatus = {
  supported: true,
  platform: "darwin",
  app: "iTerm",
  reason: null,
};

describe("resolveOverviewTapIntent", () => {
  it("routes taps to benching while bench mode is armed", () => {
    const session = makeSession({ session_id: "sess-1" });

    expect(
      resolveOverviewTapIntent({
        sessionId: "sess-1",
        sessionsList: [session],
        benchArmed: true,
        axeArmed: false,
        nativeDesktop: NATIVE_SUPPORTED,
        isObserver: false,
      }),
    ).toEqual({ type: "bench", sessionId: "sess-1" });
  });

  it("requests a refresh instead of opening stale sessions", () => {
    const stale = makeSession({ session_id: "sess-stale", is_stale: true });

    expect(
      resolveOverviewTapIntent({
        sessionId: stale.session_id,
        sessionsList: [stale],
        benchArmed: false,
        axeArmed: false,
        nativeDesktop: NATIVE_SUPPORTED,
        isObserver: false,
      }),
    ).toEqual({ type: "refresh" });
  });

  it("selects native open path when desktop open is preferred", () => {
    const session = makeSession({ session_id: "sess-native", state: "busy" });

    expect(
      resolveOverviewTapIntent({
        sessionId: session.session_id,
        sessionsList: [session],
        benchArmed: false,
        axeArmed: false,
        nativeDesktop: NATIVE_SUPPORTED,
        isObserver: false,
      }),
    ).toEqual({ type: "open-native-or-terminal", session });
  });

  it("arms delete behavior when axe mode is active", () => {
    const session = makeSession({ session_id: "sess-axe", state: "busy" });

    expect(
      resolveOverviewTapIntent({
        sessionId: session.session_id,
        sessionsList: [session],
        benchArmed: false,
        axeArmed: true,
        nativeDesktop: NATIVE_SUPPORTED,
        isObserver: false,
      }),
    ).toEqual({ type: "delete", sessionId: "sess-axe" });
  });
});

describe("deriveAppViewFlags", () => {
  it("keeps overview interactive in desktop split mode", () => {
    expect(
      deriveAppViewFlags({
        view: "terminal",
        zoneLayout: "single",
        viewportWidth: 1024,
        transport: "degraded",
      }),
    ).toEqual({
      isOverview: false,
      isTerminal: true,
      splitMode: true,
      showTransportBanner: true,
      fieldAxeTopOffset: 30,
      overviewInteractive: true,
      terminalInteractive: true,
      overviewTransform: "none",
    });
  });

  it("slides overview out in non-split terminal mode", () => {
    expect(
      deriveAppViewFlags({
        view: "terminal",
        zoneLayout: "dual",
        viewportWidth: 1200,
        transport: "healthy",
      }),
    ).toEqual({
      isOverview: false,
      isTerminal: true,
      splitMode: false,
      showTransportBanner: false,
      fieldAxeTopOffset: 8,
      overviewInteractive: false,
      terminalInteractive: true,
      overviewTransform: "translateX(-100%)",
    });
  });
});
