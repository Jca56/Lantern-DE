#!/usr/bin/env bash
# Fetch the notepad's bundled font catalog (see src/fonts.rs) AND the DE
# font families (see lntrn-theme typography + the system-settings Font
# picker) from Google Fonts into ~/.lantern/fonts/. Downloads static
# Regular / Medium / Bold / Italic / BoldItalic TTFs per family; families
# that don't ship a style (e.g. Oswald italic) are skipped automatically.
# Safe to re-run — existing files are overwritten with fresh copies.
#
# Run once per machine (laptop + PC) — binaries load these at startup and
# silently fall back to the default sans when they're missing.
set -euo pipefail

DEST="${HOME}/.lantern/fonts"
mkdir -p "$DEST"

# "Family Name|FileBase" — FileBase must match src/fonts.rs exactly.
FAMILIES=(
  "Roboto|Roboto"
  "Open Sans|OpenSans"
  "Lato|Lato"
  "Montserrat|Montserrat"
  "Poppins|Poppins"
  "Inter|Inter"
  "Nunito|Nunito"
  "Raleway|Raleway"
  "Work Sans|WorkSans"
  "DM Sans|DMSans"
  "Rubik|Rubik"
  "Quicksand|Quicksand"
  "Oswald|Oswald"
  "Source Sans 3|SourceSans3"
  "PT Sans|PTSans"
  "Lora|Lora"
  "Merriweather|Merriweather"
  "Playfair Display|PlayfairDisplay"
  "Bitter|Bitter"
  "JetBrains Mono|JetBrainsMono"
)

# DE-wide proportional font options offered by lntrn-system-settings
# (Appearance → Font). Family names must match FONT_OPTIONS in
# appearance_panel.rs. Inter is also in the notepad catalog above, but is
# listed here too so the DE set stays complete if the catalog changes.
DE_FAMILIES=(
  "Inter|Inter"
  "Lexend|Lexend"
  "Atkinson Hyperlegible|AtkinsonHyperlegible"
  "IBM Plex Sans|IBMPlexSans"
)

ok=0
skipped=0

fetch_family() {
  local family="$1" base="$2"
  local url_family="${family// /+}"

  # The css2 API serves plain static-TTF @font-face blocks to legacy user
  # agents (curl's default UA qualifies) — one block per available style.
  local css
  css=$(curl -fsS "https://fonts.googleapis.com/css2?family=${url_family}:ital,wght@0,400;0,500;0,700;1,400;1,700") || {
    echo "FAIL  ${family} (css fetch)"
    return 0
  }

  # Emit "style weight url" per @font-face block.
  local style weight url suffix out
  while read -r style weight url; do
    case "${style},${weight}" in
      normal,400) suffix="" ;;
      normal,500) suffix="-Medium" ;;
      normal,700) suffix="-Bold" ;;
      italic,400) suffix="-Italic" ;;
      italic,700) suffix="-BoldItalic" ;;
      *) continue ;;
    esac
    out="${DEST}/${base}${suffix}.ttf"
    if curl -fsS -o "$out" "$url"; then
      echo "ok    ${base}${suffix}.ttf"
      ok=$((ok + 1))
    else
      echo "FAIL  ${base}${suffix}.ttf"
    fi
  done < <(echo "$css" | awk '
    /font-style:/  { style = $2; gsub(/;/, "", style) }
    /font-weight:/ { weight = $2; gsub(/;/, "", weight) }
    /src:/ {
      if (match($0, /url\([^)]*\)/)) {
        print style, weight, substr($0, RSTART + 4, RLENGTH - 5)
      }
    }
  ')

  # Note families with no italic faces so the output explains the gaps.
  if ! echo "$css" | grep -q "font-style: italic"; then
    echo "note  ${family} ships no italic — skipped"
    skipped=$((skipped + 1))
  fi
}

for entry in "${FAMILIES[@]}" "${DE_FAMILIES[@]}"; do
  fetch_family "${entry%%|*}" "${entry##*|}"
done

echo
echo "Done: ${ok} files into ${DEST} (${skipped} families without italics)"
