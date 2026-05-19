#!/usr/bin/env python3
"""
Set up emoji assets for Lantern Command Center.

Pure Unicode-org sources:
 - emoji-test.txt → codepoint, glyph, name, category, display order
 - CLDR annotations/en.xml → keywords for search
 - googlefonts/noto-emoji → the actual PNGs

Run ONCE:
    python3 scripts/setup-emojis.py

What it does:
 1. Fetches Unicode emoji-test.txt (cached in /tmp).
 2. Fetches Unicode CLDR en.xml annotations (cached in /tmp).
 3. Shallow-clones googlefonts/noto-emoji into /tmp with sparse-checkout
    of png/128 only (for the actual PNGs).
 4. Installs PNGs to ~/.lantern/emojis/png/{unicode}.png
 5. Generates lntrn-command-center/src/emojis/data.rs, sorted by
    (category, canonical emoji-test rank).

Subsequent runs are idempotent: caches/clones reused if intact.
"""

import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

NOTO_URL = "https://github.com/googlefonts/noto-emoji.git"
NOTO_DIR = Path("/tmp/noto-emoji")
EMOJI_TEST_URL = "https://unicode.org/Public/emoji/16.0/emoji-test.txt"
EMOJI_TEST_CACHE = Path("/tmp/emoji-test-16.0.txt")
CLDR_URL = (
    "https://raw.githubusercontent.com/unicode-org/cldr/main/"
    "common/annotations/en.xml"
)
CLDR_CACHE = Path("/tmp/cldr-en-annotations.xml")
HOME = Path.home()
EMOJI_DIR = HOME / ".lantern" / "emojis" / "png"
WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
DATA_RS = WORKSPACE_ROOT / "lntrn-command-center" / "src" / "emojis" / "data.rs"

# Map Unicode CLDR group strings to compact category enum names.
# "Component" (skin tone modifiers etc.) is intentionally dropped —
# they're not standalone user-facing emojis.
GROUP_TO_CAT = {
    "Smileys & Emotion": "Smileys",
    "People & Body": "People",
    "Animals & Nature": "AnimalsNature",
    "Food & Drink": "FoodDrink",
    "Travel & Places": "TravelPlaces",
    "Activities": "Activities",
    "Objects": "Objects",
    "Symbols": "Symbols",
    "Flags": "Flags",
}

CAT_ORDER = [
    "Smileys",
    "People",
    "AnimalsNature",
    "FoodDrink",
    "Activities",
    "TravelPlaces",
    "Objects",
    "Symbols",
    "Flags",
]


def ensure_noto_clone() -> Path:
    if NOTO_DIR.exists() and (NOTO_DIR / "png" / "128").exists():
        print(f"reusing noto-emoji clone at {NOTO_DIR}")
        return NOTO_DIR
    if NOTO_DIR.exists():
        shutil.rmtree(NOTO_DIR)
    print(f"cloning {NOTO_URL} → {NOTO_DIR} (shallow, png/128 only)…")
    subprocess.run(
        ["git", "clone", "--depth", "1", "--filter=blob:none", "--sparse",
         NOTO_URL, str(NOTO_DIR)],
        check=True,
    )
    subprocess.run(
        ["git", "-C", str(NOTO_DIR), "sparse-checkout", "set", "png/128"],
        check=True,
    )
    return NOTO_DIR


def fetch_cached(url: str, cache: Path) -> Path:
    """Curl `url` into `cache` if missing/empty; otherwise reuse."""
    if cache.exists() and cache.stat().st_size > 0:
        print(f"reusing {cache.name} at {cache}")
        return cache
    print(f"fetching {url} …")
    subprocess.run(
        ["curl", "-fsSL", "-o", str(cache), url],
        check=True,
    )
    return cache


def parse_emoji_test(path: Path) -> list[dict]:
    """Parse Unicode emoji-test.txt → ordered list of entries.

    Each entry has: unicode (space-sep lowercase hex, FE0F stripped),
    glyph (the fully-qualified display string), name, category, rank
    (insertion order = canonical Unicode display order)."""
    entries: list[dict] = []
    current_group: str | None = None
    rank = 0
    for line in path.read_text().splitlines():
        if line.startswith("# group:"):
            current_group = line.split(":", 1)[1].strip()
            continue
        if not line or line.startswith("#"):
            continue
        if ";" not in line or "#" not in line:
            continue
        head, rest = line.split(";", 1)
        status, comment = rest.split("#", 1)
        if status.strip() != "fully-qualified":
            continue
        cps = head.strip().lower().split()
        # Strip variation selectors — Noto's filenames for our subset
        # (single codepoint + regional-indicator flag pairs) don't use
        # them, and our on-disk PNG names follow that convention.
        cps_stripped = [cp for cp in cps if cp != "fe0f"]
        if not cps_stripped:
            continue
        # comment format: " <glyph> EX.Y <name>"
        parts = comment.strip().split(None, 2)
        if len(parts) < 3:
            continue
        glyph, _version, name = parts[0], parts[1], parts[2]
        cat = GROUP_TO_CAT.get(current_group or "")
        if not cat:
            continue
        entries.append({
            "unicode": " ".join(cps_stripped),
            "glyph": glyph,
            "name": name,
            "category": cat,
            "rank": rank,
        })
        rank += 1
    return entries


def parse_cldr_keywords(path: Path) -> dict[str, list[str]]:
    """Parse CLDR annotations/en.xml → {emoji glyph → [keywords]}.

    Skips entries with type="tts" (those are TTS names, not keywords)."""
    tree = ET.parse(path)
    root = tree.getroot()
    keywords: dict[str, list[str]] = {}
    for ann in root.iter("annotation"):
        cp = ann.get("cp")
        if not cp or ann.get("type") == "tts":
            continue
        text = (ann.text or "").strip()
        if not text:
            continue
        kws = [k.strip() for k in text.split("|") if k.strip()]
        if kws:
            keywords[cp] = kws
    return keywords


def noto_png_for(unicode_str: str, noto_root: Path) -> Path | None:
    """Map space-sep lowercase hex to the matching PNG in noto's png/128.

    Noto names files like `emoji_u1f600.png` and `emoji_u1f1fa_1f1f8.png`
    for sequences."""
    parts = unicode_str.split()
    fname = "emoji_u" + "_".join(parts) + ".png"
    p = noto_root / "png" / "128" / fname
    return p if p.exists() else None


def rust_string(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main() -> int:
    if not shutil.which("git"):
        print("error: git is required", file=sys.stderr)
        return 1
    if not shutil.which("curl"):
        print("error: curl is required", file=sys.stderr)
        return 1

    noto = ensure_noto_clone()
    emoji_entries = parse_emoji_test(fetch_cached(EMOJI_TEST_URL, EMOJI_TEST_CACHE))
    keywords_map = parse_cldr_keywords(fetch_cached(CLDR_URL, CLDR_CACHE))
    print(
        f"parsed {len(emoji_entries)} emoji entries, "
        f"{len(keywords_map)} CLDR annotations"
    )

    EMOJI_DIR.mkdir(parents=True, exist_ok=True)
    # Wipe stale PNGs from previous runs (e.g. older emoji sets).
    for old in EMOJI_DIR.glob("*.png"):
        old.unlink()
    # Also wipe the legacy 3d/ dir from earlier runs.
    legacy = HOME / ".lantern" / "emojis" / "3d"
    if legacy.exists():
        shutil.rmtree(legacy)

    out_entries: list[dict] = []
    skipped_no_png = 0
    skipped_multi = 0

    for e in emoji_entries:
        parts = e["unicode"].split()
        # Skip multi-codepoint emoji (skin tones, ZWJ sequences) for
        # v1 simplicity. Keep simple flags ("1f1fa 1f1f8" → two regional
        # indicators).
        if len(parts) > 1:
            if not all(p.startswith("1f1") for p in parts):
                skipped_multi += 1
                continue
        png = noto_png_for(e["unicode"], noto)
        if not png:
            skipped_no_png += 1
            continue
        out_name = e["unicode"].replace(" ", "-") + ".png"
        out = EMOJI_DIR / out_name
        if not out.exists() or out.stat().st_size != png.stat().st_size:
            shutil.copyfile(png, out)
        kws = keywords_map.get(e["glyph"], [])
        out_entries.append({
            "unicode": e["unicode"],
            "glyph": e["glyph"],
            "name": e["name"],
            "category": e["category"],
            "keywords": kws,
            "rank": e["rank"],
        })

    print(
        f"installed {len(out_entries)} emojis → {EMOJI_DIR}  "
        f"(skipped: {skipped_no_png} missing PNG, {skipped_multi} multi-codepoint)"
    )

    out_entries.sort(key=lambda e: (CAT_ORDER.index(e["category"]), e["rank"]))

    DATA_RS.parent.mkdir(parents=True, exist_ok=True)
    with DATA_RS.open("w") as f:
        f.write("//! Auto-generated by scripts/setup-emojis.py.  DO NOT EDIT BY HAND.\n")
        f.write("//! Re-run the script after refreshing Unicode/Noto sources to regenerate.\n\n")
        f.write("use super::Category;\n\n")
        f.write("/// One emoji entry: codepoint, display glyph, name, category, keywords.\n")
        f.write("pub struct EmojiEntry {\n")
        f.write("    pub unicode: &'static str,\n")
        f.write("    pub glyph: &'static str,\n")
        f.write("    pub name: &'static str,\n")
        f.write("    pub category: Category,\n")
        f.write("    pub keywords: &'static [&'static str],\n")
        f.write("}\n\n")
        f.write("pub const EMOJIS: &[EmojiEntry] = &[\n")
        for e in out_entries:
            kws = ", ".join(rust_string(k) for k in e["keywords"])
            f.write("    EmojiEntry {\n")
            f.write(f"        unicode: {rust_string(e['unicode'])},\n")
            f.write(f"        glyph: {rust_string(e['glyph'])},\n")
            f.write(f"        name: {rust_string(e['name'])},\n")
            f.write(f"        category: Category::{e['category']},\n")
            f.write(f"        keywords: &[{kws}],\n")
            f.write("    },\n")
        f.write("];\n")

    print(f"wrote {DATA_RS} with {len(out_entries)} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main())
