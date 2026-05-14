#!/usr/bin/env python3
"""
Set up emoji assets for Lantern Command Center.

Uses Microsoft Fluent UI Emoji for the rich CLDR metadata (category,
keywords, names) and Google Noto Color Emoji for the actual PNGs.

Run ONCE:
    python3 scripts/setup-emojis.py

What it does:
 1. Shallow-clones microsoft/fluentui-emoji into /tmp (for metadata only).
 2. Shallow-clones googlefonts/noto-emoji into /tmp with sparse-checkout
    of png/128 only (for the actual PNGs).
 3. Installs PNGs to ~/.lantern/emojis/png/{unicode}.png
 4. Generates lntrn-command-center/src/emojis/data.rs with the table,
    sorted by (category, codepoint) for natural Unicode order.

Subsequent runs are idempotent: existing clones reused if intact.
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path

FLUENT_URL = "https://github.com/microsoft/fluentui-emoji.git"
FLUENT_DIR = Path("/tmp/fluentui-emoji")
NOTO_URL = "https://github.com/googlefonts/noto-emoji.git"
NOTO_DIR = Path("/tmp/noto-emoji")
# Canonical Unicode emoji display order (face-smiling, face-affection, …,
# then hearts, then monsters, etc.) used to sort within each category.
EMOJI_TEST_URL = "https://unicode.org/Public/emoji/16.0/emoji-test.txt"
EMOJI_TEST_CACHE = Path("/tmp/emoji-test-16.0.txt")
HOME = Path.home()
EMOJI_DIR = HOME / ".lantern" / "emojis" / "png"
WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
DATA_RS = WORKSPACE_ROOT / "lntrn-command-center" / "src" / "emojis" / "data.rs"

# Map Fluent's CLDR group strings to compact category enum names.
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


def ensure_fluent_clone() -> Path:
    if FLUENT_DIR.exists() and (FLUENT_DIR / "assets").exists():
        print(f"reusing fluentui-emoji clone at {FLUENT_DIR}")
        return FLUENT_DIR
    if FLUENT_DIR.exists():
        shutil.rmtree(FLUENT_DIR)
    print(f"cloning {FLUENT_URL} → {FLUENT_DIR} (shallow, metadata only)…")
    subprocess.run(
        ["git", "clone", "--depth", "1", "--filter=blob:none", "--sparse",
         FLUENT_URL, str(FLUENT_DIR)],
        check=True,
    )
    # Only pull the metadata.json files — we don't need any PNGs from
    # this repo.
    subprocess.run(
        ["git", "-C", str(FLUENT_DIR), "sparse-checkout", "set", "assets"],
        check=True,
    )
    return FLUENT_DIR


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


def fetch_emoji_test() -> Path:
    """Download Unicode's emoji-test.txt (cached). Defines display order."""
    if EMOJI_TEST_CACHE.exists() and EMOJI_TEST_CACHE.stat().st_size > 0:
        print(f"reusing emoji-test cache at {EMOJI_TEST_CACHE}")
        return EMOJI_TEST_CACHE
    print(f"fetching {EMOJI_TEST_URL} …")
    subprocess.run(
        ["curl", "-fsSL", "-o", str(EMOJI_TEST_CACHE), EMOJI_TEST_URL],
        check=True,
    )
    return EMOJI_TEST_CACHE


def parse_emoji_order(path: Path) -> dict[str, int]:
    """Return {space-separated-lowercase-hex: rank}. Lower rank = earlier
    in canonical display order. Only fully-qualified entries are kept —
    those without variation selectors / qualifiers we strip down to the
    base codepoints so unmatched lookups can fall back."""
    order: dict[str, int] = {}
    rank = 0
    for line in path.read_text().splitlines():
        # Skip comments / blank.
        if not line or line.startswith("#"):
            continue
        # Format: <hex sequence> ; <status> # <emoji> EX.Y <name>
        head = line.split(";", 1)[0].strip()
        if not head:
            continue
        # Sequence is space-separated uppercase hex like "1F1FA 1F1F8".
        cps = head.lower().split()
        # Strip variation selectors (FE0F) — Fluent's "unicode" never
        # includes them, so matching needs them removed.
        cps = [cp for cp in cps if cp != "fe0f"]
        if not cps:
            continue
        key = " ".join(cps)
        if key not in order:
            order[key] = rank
            rank += 1
    return order


def noto_png_for(unicode_str: str, noto_root: Path) -> Path | None:
    """Map Fluent's unicode string (e.g. "1f600" or "1f1fa 1f1f8") to the
    matching PNG inside noto-emoji's png/128 directory.

    Noto names files like `emoji_u1f600.png` and `emoji_u1f1fa_1f1f8.png`
    for sequences."""
    parts = unicode_str.split()
    fname = "emoji_u" + "_".join(parts) + ".png"
    p = noto_root / "png" / "128" / fname
    return p if p.exists() else None


def rust_string(s: str) -> str:
    # Quote a string for inclusion in a Rust string literal.
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main() -> int:
    if not shutil.which("git"):
        print("error: git is required", file=sys.stderr)
        return 1

    fluent = ensure_fluent_clone()
    noto = ensure_noto_clone()
    order = parse_emoji_order(fetch_emoji_test())
    print(f"loaded canonical order with {len(order)} entries")
    assets = fluent / "assets"
    if not assets.is_dir():
        print(f"error: {assets} not found", file=sys.stderr)
        return 1

    EMOJI_DIR.mkdir(parents=True, exist_ok=True)
    # Wipe stale PNGs from previous emoji-set runs (e.g. Fluent 3D).
    for old in EMOJI_DIR.glob("*.png"):
        old.unlink()
    # Also wipe the legacy 3d/ dir from earlier runs.
    legacy = HOME / ".lantern" / "emojis" / "3d"
    if legacy.exists():
        shutil.rmtree(legacy)

    # entries: list of dicts with keys unicode, glyph, name, category, keywords
    entries: list[dict] = []
    skipped_no_png = 0
    skipped_no_cat = 0
    multi_codepoint = 0

    for emoji_dir in sorted(assets.iterdir()):
        if not emoji_dir.is_dir():
            continue
        meta_path = emoji_dir / "metadata.json"
        if not meta_path.exists():
            continue
        try:
            meta = json.loads(meta_path.read_text())
        except Exception as e:
            print(f"warn: bad metadata in {meta_path}: {e}", file=sys.stderr)
            continue

        unicode_str = meta.get("unicode", "").strip().lower()
        if not unicode_str:
            continue
        # Skip multi-codepoint emoji (skin tones, ZWJ sequences, flags-of-...) for v1 simplicity.
        # Keep simple flags ("1F1FA 1F1F8" → length 2 with both regional indicators).
        parts = unicode_str.split()
        if len(parts) > 1:
            multi_codepoint += 1
            # Allow regional indicator pairs (flags) only.
            if not all(p.startswith("1f1") for p in parts):
                continue
        group = meta.get("group", "")
        category = GROUP_TO_CAT.get(group)
        if not category:
            skipped_no_cat += 1
            continue

        cldr = meta.get("cldr", emoji_dir.name)
        png = noto_png_for(unicode_str, noto)
        if not png:
            skipped_no_png += 1
            continue

        # Filename normalised: codepoint-with-dashes-for-multi.
        out_name = unicode_str.replace(" ", "-") + ".png"
        out = EMOJI_DIR / out_name
        if not out.exists() or out.stat().st_size != png.stat().st_size:
            shutil.copyfile(png, out)

        entries.append({
            "unicode": unicode_str,
            "glyph": meta.get("glyph", ""),
            "name": cldr,
            "category": category,
            "keywords": meta.get("keywords", []),
        })

    print(
        f"installed {len(entries)} emojis → {EMOJI_DIR}  "
        f"(skipped: {skipped_no_png} missing PNG, {skipped_no_cat} no category, "
        f"{multi_codepoint} multi-codepoint)"
    )

    # Fall-back rank for any emoji missing from emoji-test.txt — push
    # them after everything that does have a canonical rank.
    UNRANKED = 10_000_000

    def sort_key(e):
        cat_idx = CAT_ORDER.index(e["category"])
        rank = order.get(e["unicode"], UNRANKED)
        # Tie-break on first codepoint so unranked entries stay grouped.
        cp = int(e["unicode"].split()[0], 16)
        return (cat_idx, rank, cp)

    entries.sort(key=sort_key)

    DATA_RS.parent.mkdir(parents=True, exist_ok=True)
    with DATA_RS.open("w") as f:
        f.write("//! Auto-generated by scripts/setup-emojis.py.  DO NOT EDIT BY HAND.\n")
        f.write("//! Re-run the script after refreshing Fluent UI Emoji to regenerate.\n\n")
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
        for e in entries:
            kws = ", ".join(rust_string(k) for k in e["keywords"])
            f.write("    EmojiEntry {\n")
            f.write(f"        unicode: {rust_string(e['unicode'])},\n")
            f.write(f"        glyph: {rust_string(e['glyph'])},\n")
            f.write(f"        name: {rust_string(e['name'])},\n")
            f.write(f"        category: Category::{e['category']},\n")
            f.write(f"        keywords: &[{kws}],\n")
            f.write("    },\n")
        f.write("];\n")

    print(f"wrote {DATA_RS} with {len(entries)} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main())
