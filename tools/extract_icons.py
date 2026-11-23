"""Extracts Sekiro's combat art icons into PNGs that the Drawn Arts wheel loads.

Unpack these from the game first (UXM to unpack the game, WitchyBND for the archives):
  menu/hi/01_common.tpf.dcx       -> a folder of .dds texture atlases
  menu/hi/01_common.sblytbnd.dcx  -> a folder of .layout files describing the atlases

Then run (needs Pillow: `pip install pillow`):
  python tools/extract_icons.py <textures folder> <layouts folder> <output folder>

For example:
  python tools/extract_icons.py menu/hi/01_common-tpf-dcx menu/hi/01_common-sblytbnd-dcx drawn_arts/icons
"""

import sys
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image

# Combat art uid -> iconId, from EquipParamWeapon in Sekiro 1.06.
ICON_IDS = {
    5100: 1370,  # Whirlwind Slash
    5200: 1440,  # Nightjar Slash
    7000: 1450,  # Nightjar Slash Reversal
    5300: 1510,  # Ichimonji
    7100: 1520,  # Ichimonji: Double
    5500: 1530,  # Ashina Cross
    5600: 1760,  # Floating Passage
    7200: 1710,  # Spiral Cloud Passage
    5900: 1600,  # Praying Strikes
    7500: 1610,  # Praying Strikes - Exorcism
    5800: 1620,  # Senpou Leaping Kicks
    7400: 1630,  # High Monk
    6000: 1380,  # Shadowrush
    7600: 1700,  # Shadowfall
    7700: 1650,  # Sakura Dance
    5700: 1770,  # Mortal Draw
    7300: 1720,  # Empowered Mortal Draw
    5400: 1750,  # Dragon Flash
    6100: 1780,  # One Mind
}


def find_icons(layouts: Path) -> dict[int, tuple[str, tuple[int, int, int, int]]]:
    """Maps each iconId found in the layouts to its atlas name and pixel box."""
    found = {}
    for layout in sorted(layouts.glob("*.layout")):
        atlas = layout.stem
        for sub in ET.parse(layout).getroot().iter("SubTexture"):
            suffix = Path(sub.get("name", "")).stem.removeprefix("MENU_ItemIcon_")
            # Skip other images and variants like "MENU_ItemIcon_00000d".
            if suffix == sub.get("name") or not suffix.isdigit():
                continue
            icon_id = int(suffix)
            x, y = int(sub.get("x")), int(sub.get("y"))
            box = (x, y, x + int(sub.get("width")), y + int(sub.get("height")))
            found.setdefault(icon_id, (atlas, box))
    return found


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__)
        return 2
    textures, layouts, out = map(Path, sys.argv[1:])
    out.mkdir(parents=True, exist_ok=True)

    icons = find_icons(layouts)
    atlases: dict[str, Image.Image] = {}
    missing = []
    for uid, icon_id in ICON_IDS.items():
        if icon_id not in icons:
            missing.append(uid)
            continue
        atlas, box = icons[icon_id]
        if atlas not in atlases:
            atlases[atlas] = Image.open(textures / f"{atlas}.dds").convert("RGBA")
        atlases[atlas].crop(box).save(out / f"{uid}.png")
        print(f"{uid}.png  <- {atlas} icon {icon_id}")

    if missing:
        print(f"no icon found for: {', '.join(map(str, missing))}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
