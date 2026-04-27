#!/usr/bin/env bash
# Set up local-AI prerequisites for the lntrn-code AI panel:
#   1. Install ollama + zram-generator from pacman
#   2. Configure ~ram-sized zstd zram swap (high priority)
#   3. Create a 32 GiB /swapfile (low priority — for LLM headroom)
#   4. Persist both to /etc/fstab + /etc/systemd/zram-generator.conf
#
# Safe to re-run: every step checks existing state before changing it.
# Run as: ./scripts/ai-setup.sh   (script will sudo internally)

set -euo pipefail

SWAPFILE=/swapfile
SWAPFILE_SIZE_GIB=32
ZRAM_CONF=/etc/systemd/zram-generator.conf

c_blue=$'\e[1;34m'; c_green=$'\e[1;32m'; c_yellow=$'\e[1;33m'; c_off=$'\e[0m'
say() { printf '%s==>%s %s\n' "$c_blue" "$c_off" "$*"; }
ok()  { printf '%s ✓%s %s\n' "$c_green" "$c_off" "$*"; }
warn(){ printf '%s !!%s %s\n' "$c_yellow" "$c_off" "$*"; }

if [[ $EUID -eq 0 ]]; then
    echo "Run as your normal user — the script sudos when it needs to." >&2
    exit 1
fi

# ── 1. Pacman packages ─────────────────────────────────────────────────────
say "Checking pacman packages (ollama, zram-generator)"
pkgs=()
pacman -Qi ollama          >/dev/null 2>&1 || pkgs+=(ollama)
pacman -Qi zram-generator  >/dev/null 2>&1 || pkgs+=(zram-generator)
if (( ${#pkgs[@]} )); then
    say "Installing: ${pkgs[*]}"
    sudo pacman -S --needed --noconfirm "${pkgs[@]}"
else
    ok "ollama + zram-generator already installed"
fi

# ── 2. zram swap (compressed RAM swap, high priority) ──────────────────────
say "Configuring zram (ram-sized, zstd)"
if [[ -f $ZRAM_CONF ]] && grep -q 'zram-size' "$ZRAM_CONF"; then
    ok "$ZRAM_CONF already present — leaving as-is"
else
    sudo tee "$ZRAM_CONF" >/dev/null <<'EOF'
[zram0]
zram-size = ram
compression-algorithm = zstd
swap-priority = 100
EOF
    ok "Wrote $ZRAM_CONF"
    sudo systemctl daemon-reload
    sudo systemctl start systemd-zram-setup@zram0.service || true
fi

# ── 3. 32 GiB swapfile (slow LLM-overflow swap, low priority) ──────────────
say "Ensuring ${SWAPFILE_SIZE_GIB} GiB ${SWAPFILE}"
if [[ -f $SWAPFILE ]]; then
    actual_gib=$(( $(stat -c %s "$SWAPFILE") / 1024 / 1024 / 1024 ))
    if (( actual_gib >= SWAPFILE_SIZE_GIB )); then
        ok "$SWAPFILE already exists (${actual_gib} GiB)"
    else
        warn "$SWAPFILE exists but is only ${actual_gib} GiB (< ${SWAPFILE_SIZE_GIB})"
        warn "Leaving alone — remove it manually with 'sudo swapoff $SWAPFILE && sudo rm $SWAPFILE' if you want to recreate"
    fi
else
    say "Allocating ${SWAPFILE_SIZE_GIB} GiB (this can take a minute)"
    sudo fallocate -l "${SWAPFILE_SIZE_GIB}G" "$SWAPFILE" \
        || sudo dd if=/dev/zero of="$SWAPFILE" bs=1M count=$((SWAPFILE_SIZE_GIB * 1024)) status=progress
    sudo chmod 600 "$SWAPFILE"
    sudo mkswap "$SWAPFILE"
    ok "Created $SWAPFILE"
fi

if ! swapon --show=NAME --noheadings | grep -qx "$SWAPFILE"; then
    sudo swapon --priority 10 "$SWAPFILE"
    ok "Enabled $SWAPFILE (priority 10)"
fi

# ── 4. Persist swapfile in /etc/fstab ─────────────────────────────────────
if ! grep -qE "^${SWAPFILE//\//\\/}\s" /etc/fstab; then
    echo "$SWAPFILE none swap defaults,pri=10 0 0" | sudo tee -a /etc/fstab >/dev/null
    ok "Added $SWAPFILE to /etc/fstab"
else
    ok "$SWAPFILE already in /etc/fstab"
fi

# ── 5. Summary ─────────────────────────────────────────────────────────────
echo
say "Done. Current swap:"
swapon --show
echo
say "Next steps (run manually when ready):"
cat <<EOF
  # Start the daemon (on-demand mode — no systemd service)
  ollama serve &

  # Pull benchmark candidates (each download is multi-GB)
  ollama pull qwen2.5-coder:7b      # safe baseline (~4.5 GB)
  ollama pull qwen2.5-coder:14b     # dense, fits in 14 GB RAM (~9 GB)
  ollama pull qwen3-coder-next      # MoE wildcard (~45 GB on disk)
  # ollama pull qwen3.6:27b         # only if you want to feel pain on the laptop

  # Quick speed check (look at the eval rate / tok/s line)
  ollama run qwen2.5-coder:14b --verbose "Write a Rust function that reverses a string."
EOF
