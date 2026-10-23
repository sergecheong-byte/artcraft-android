#!/usr/bin/env python3
"""Remove black edge pixels connected to the outside of DeckCraft's approved Dk icon.

This is Android launcher artwork normalization, not an edit to the source master.
Preserves non-edge lettering and internal glass highlights.
"""
from collections import deque
from pathlib import Path
from PIL import Image
import sys

def normalize(path: Path) -> None:
    with Image.open(path) as image:
        img = image.convert("RGBA")
    w, h = img.size
    if (w, h) != (192, 192):
        raise SystemExit(f"Unexpected icon dimensions: {w}x{h}")
    px = img.load()
    # Estimate a deep-red background from highly red, non-highlight pixels.
    candidates = []
    for y in range(h // 4, (3 * h) // 4):
        for x in range(w // 4, (3 * w) // 4):
            r, g, b, a = px[x, y]
            if a > 240 and r > g * 1.7 and r > b * 1.5 and 45 < r < 180:
                candidates.append((r, g, b))
    if len(candidates) < 25:
        raise SystemExit("Cannot identify approved red icon background; refusing image change")
    candidates.sort()
    fill = candidates[len(candidates) // 3] + (255,)
    def edge_dark(x, y):
        r, g, b, a = px[x, y]
        return a < 50 or (max(r, g, b) < 105 and (max(r,g,b) - min(r,g,b) < 65))
    q = deque()
    seen = set()
    for x in range(w):
        for y in (0, h-1):
            if edge_dark(x,y):
                q.append((x,y))
                seen.add((x,y))
    for y in range(h):
        for x in (0,w-1):
            if edge_dark(x,y):
                q.append((x,y))
                seen.add((x,y))
    while q:
        x,y=q.popleft()
        px[x,y]=fill
        for nx,ny in ((x+1,y),(x-1,y),(x,y+1),(x,y-1)):
            if 0<=nx<w and 0<=ny<h and (nx,ny) not in seen and edge_dark(nx,ny):
                seen.add((nx,ny))
                q.append((nx,ny))
    if len(seen) < 150:
        raise SystemExit("Expected dark outer border was not detected; refusing icon change")
    # Replace dark edge-connected surround with deep-red background only.
    img.save(path, optimize=True)
    print(f"Normalized {len(seen)} outer-border pixels; glass lettering retained")

if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: normalize_deckcraft_icon.py approved.png")
    normalize(Path(sys.argv[1]))
