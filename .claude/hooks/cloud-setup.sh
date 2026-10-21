#!/usr/bin/env bash
# Cloud sessions only: the system packages Bevy needs to build and the CPU Vulkan driver (lavapipe) that runs the headless shots and
# gates without a GPU. The Rust toolchain comes from rust-toolchain.toml. Idempotent; local sessions exit at once.
[ "$CLAUDE_CODE_REMOTE" = "true" ] || exit 0
need=""
for p in pkg-config libwayland-dev libasound2-dev libudev-dev libxkbcommon-dev mesa-vulkan-drivers libvulkan1; do
  dpkg -s "$p" >/dev/null 2>&1 || need="$need $p"
done
if [ -n "$need" ]; then
  (apt-get install -y -q $need || (apt-get update -q && apt-get install -y -q $need)) >/tmp/cloud-setup-apt.log 2>&1 || echo "cloud-setup: apt failed, see /tmp/cloud-setup-apt.log"
fi
icd=/usr/share/vulkan/icd.d/lvp_icd.json
[ -f "$icd" ] && [ -n "$CLAUDE_ENV_FILE" ] && echo "export VK_ICD_FILENAMES=$icd" >> "$CLAUDE_ENV_FILE"
# Dependencies in the background: the first cargo command then starts warm.
(cd "$CLAUDE_PROJECT_DIR" && cargo fetch -q >/tmp/cloud-setup-fetch.log 2>&1 &)
exit 0
