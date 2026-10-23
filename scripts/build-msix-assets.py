#!/usr/bin/env python3
"""Generates the Microsoft Store (MSIX) visual assets from the app icon.

    pip install pillow
    python3 scripts/build-msix-assets.py

Writes packaging/windows/msix/Assets/*.png. Each logo is written at scale 100 and 200 plus
target sizes for the taskbar/Start icon; scripts/package-msix.ps1 indexes them with makepri.
"""
import pathlib

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
ICON = Image.open(ROOT / "packaging" / "icons" / "scratchpad-1024.png").convert("RGBA")
OUT = ROOT / "packaging" / "windows" / "msix" / "Assets"
OUT.mkdir(parents=True, exist_ok=True)
for old in OUT.glob("*.png"):
    old.unlink()


def logo(width, height, icon_ratio):
    """The icon centred on a transparent canvas, `icon_ratio` of the shorter side."""
    canvas = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    side = round(min(width, height) * icon_ratio)
    icon = ICON.resize((side, side), Image.LANCZOS)
    canvas.paste(icon, ((width - side) // 2, (height - side) // 2), icon)
    return canvas


# name: (width, height at scale 100, icon size relative to the shorter side)
LOGOS = {
    "StoreLogo": (50, 50, 1.0),
    "Square44x44Logo": (44, 44, 1.0),
    "Square71x71Logo": (71, 71, 0.6),
    "Square150x150Logo": (150, 150, 0.6),
    "Wide310x150Logo": (310, 150, 0.6),
    "Square310x310Logo": (310, 310, 0.6),
    "SplashScreen": (620, 300, 0.5),
}
for name, (w, h, ratio) in LOGOS.items():
    for scale in (100, 200):
        f = scale / 100
        logo(round(w * f), round(h * f), ratio).save(OUT / f"{name}.scale-{scale}.png")

# Taskbar, Start menu and Alt-Tab use target-size icons without the tile plate.
for size in (16, 24, 32, 48, 256):
    icon = logo(size, size, 1.0)
    icon.save(OUT / f"Square44x44Logo.targetsize-{size}.png")
    icon.save(OUT / f"Square44x44Logo.targetsize-{size}_altform-unplated.png")
    icon.save(OUT / f"Square44x44Logo.targetsize-{size}_altform-lightunplated.png")

print(f"{len(list(OUT.glob('*.png')))} assets in {OUT.relative_to(ROOT)}")
