#!/usr/bin/env python3
"""Color-key the dragon sprite PNGs so only cream linework + warm flame stay opaque.

Mirrors the algorithm from the Dragon Walk-around design prototype
(`keyOutBackground` in dragon-project/project/game.jsx): luma threshold,
3x3 morphological erode, then double dilate to fatten the lines back up.
The result replaces the source PNGs in place with RGBA versions.

Run with:  python3 scripts/key-dragon-sprites.py
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError as e:  # pragma: no cover
    print("Pillow is required: pip install Pillow", file=sys.stderr)
    raise SystemExit(1) from e

# Tunables match the prototype.
LUM_THRESHOLD = 130   # cream linework lumas land 140-200; specks <= ~115.
WARM_THRESHOLD = 40   # flame: red-channel dominance over blue.

ASSET_ROOT = Path(__file__).resolve().parent.parent / "assets" / "dragon"
POSES = [
    "mouth-closed",
    "mouth-open",
    "fire-left-short",
    "fire-left-mid",
    "fire-left-full",
    "fire-right-short",
    "fire-right-mid",
    "fire-right-full",
]
FRAMES = [
    "left.png",
    "right.png",
    "front.png",
    "back.png",
    "3q-left.png",
    "3q-right.png",
    "back-left.png",
    "back-right.png",
]


def key_image(rgb: bytes, w: int, h: int) -> list[int]:
    """Return a 0/1 mask of opaque pixels for the WxH RGB byte buffer."""
    mask = bytearray(w * h)
    for i in range(w * h):
        r, g, b = rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]
        lum = 0.299 * r + 0.587 * g + 0.114 * b
        warm = r - b
        mask[i] = 1 if lum >= LUM_THRESHOLD or warm >= WARM_THRESHOLD else 0
    return mask


def erode(mask: bytearray, w: int, h: int, min_neighbors: int = 5) -> bytearray:
    """Soft erode: an on pixel survives if it itself is on AND at least
    `min_neighbors` of its 8 neighbors are on. The prototype's strict
    3x3 erode (require all 9) was too harsh on the swimmers assets,
    where many dragon lines are only 1–2 px wide and would vanish."""
    out = bytearray(w * h)
    for y in range(1, h - 1):
        row = y * w
        for x in range(1, w - 1):
            i = row + x
            if not mask[i]:
                continue
            cnt = 0
            for dy in (-w, 0, w):
                for dx in (-1, 0, 1):
                    if dx == 0 and dy == 0:
                        continue
                    if mask[i + dy + dx]:
                        cnt += 1
            if cnt >= min_neighbors:
                out[i] = 1
    return out


def dilate(mask: bytearray, w: int, h: int) -> bytearray:
    out = bytearray(w * h)
    for y in range(1, h - 1):
        row = y * w
        for x in range(1, w - 1):
            i = row + x
            any_on = 0
            for dy in (-w, 0, w):
                if any_on:
                    break
                for dx in (-1, 0, 1):
                    if mask[i + dy + dx]:
                        any_on = 1
                        break
            out[i] = any_on
    return out


def process(path: Path) -> bool:
    img = Image.open(path).convert("RGB")
    w, h = img.size
    rgb = img.tobytes()

    mask = key_image(rgb, w, h)
    mask = erode(mask, w, h)
    mask = dilate(mask, w, h)
    mask = dilate(mask, w, h)

    rgba = bytearray(w * h * 4)
    for i in range(w * h):
        if mask[i]:
            rgba[i * 4] = rgb[i * 3]
            rgba[i * 4 + 1] = rgb[i * 3 + 1]
            rgba[i * 4 + 2] = rgb[i * 3 + 2]
            rgba[i * 4 + 3] = 255
        # else leave RGBA as 0,0,0,0
    out = Image.frombytes("RGBA", (w, h), bytes(rgba))
    out.save(path, format="PNG", optimize=True)
    return True


def main() -> int:
    if not ASSET_ROOT.exists():
        print(f"asset root not found: {ASSET_ROOT}", file=sys.stderr)
        return 1
    total = 0
    keyed = 0
    for pose in POSES:
        pose_dir = ASSET_ROOT / pose
        if not pose_dir.is_dir():
            print(f"skip missing pose dir: {pose_dir}", file=sys.stderr)
            continue
        for frame in FRAMES:
            path = pose_dir / frame
            if not path.exists():
                continue
            total += 1
            if process(path):
                keyed += 1
    print(f"keyed {keyed}/{total} sprites under {ASSET_ROOT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
