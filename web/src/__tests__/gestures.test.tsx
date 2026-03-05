import { describe, expect, it, vi } from "vitest";
import { render, fireEvent } from "@testing-library/preact";
import { h } from "preact";
import { useRef } from "preact/hooks";
import {
  isHeaderSwipeCloseGesture,
  isSwipeBackGesture,
  useHeaderSwipeClose,
  useSwipeBack,
} from "@/hooks/useGestures";

function HeaderSwipeHarness({ onClose }: { onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useHeaderSwipeClose(ref, onClose);
  return <div ref={ref} data-testid="header" />;
}

function SwipeBackHarness({ onBack }: { onBack: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useSwipeBack(ref, onBack);
  return <div ref={ref} data-testid="wrapper" />;
}

describe("gesture classifiers", () => {
  it("accepts edge swipe-right as back", () => {
    expect(isSwipeBackGesture(20, 130)).toBe(true);
  });

  it("rejects back swipe when start is outside edge zone", () => {
    expect(isSwipeBackGesture(60, 180)).toBe(false);
  });

  it("accepts qualifying header swipe-left close", () => {
    expect(
      isHeaderSwipeCloseGesture({
        startX: 320,
        startY: 100,
        endX: 120,
        endY: 118,
        durationMs: 350,
      }),
    ).toBe(true);
  });

  it("rejects header close swipe from left-edge back zone", () => {
    expect(
      isHeaderSwipeCloseGesture({
        startX: 20,
        startY: 100,
        endX: -140,
        endY: 110,
        durationMs: 240,
      }),
    ).toBe(false);
  });

  it("rejects header close swipe with high vertical movement", () => {
    expect(
      isHeaderSwipeCloseGesture({
        startX: 320,
        startY: 100,
        endX: 120,
        endY: 131,
        durationMs: 200,
      }),
    ).toBe(false);
  });

  it("rejects header close swipe that is too slow", () => {
    expect(
      isHeaderSwipeCloseGesture({
        startX: 320,
        startY: 100,
        endX: 120,
        endY: 110,
        durationMs: 351,
      }),
    ).toBe(false);
  });
});

describe("gesture hooks", () => {
  it("fires header swipe-close callback for a qualifying left swipe", () => {
    const onClose = vi.fn();
    const { getByTestId } = render(<HeaderSwipeHarness onClose={onClose} />);
    const header = getByTestId("header");

    fireEvent.touchStart(header, {
      touches: [{ clientX: 320, clientY: 120 }],
    });
    fireEvent.touchEnd(header, {
      changedTouches: [{ clientX: 120, clientY: 132 }],
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not close header for a short tap-like move", () => {
    const onClose = vi.fn();
    const { getByTestId } = render(<HeaderSwipeHarness onClose={onClose} />);
    const header = getByTestId("header");

    fireEvent.touchStart(header, {
      touches: [{ clientX: 250, clientY: 120 }],
    });
    fireEvent.touchEnd(header, {
      changedTouches: [{ clientX: 240, clientY: 122 }],
    });

    expect(onClose).not.toHaveBeenCalled();
  });

  it("keeps existing edge swipe-back behavior", () => {
    const onBack = vi.fn();
    const { getByTestId } = render(<SwipeBackHarness onBack={onBack} />);
    const wrapper = getByTestId("wrapper");

    fireEvent.touchStart(wrapper, {
      touches: [{ clientX: 20, clientY: 200 }],
    });
    fireEvent.touchEnd(wrapper, {
      changedTouches: [{ clientX: 130, clientY: 202 }],
    });

    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
