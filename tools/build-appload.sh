#!/usr/bin/env bash
# Build the reMarkable AppLoad bundle for OpenRouter Chat.
#
# Produces:
#   dist/openrouter-chat/                  installable app directory
#     openrouter-chat                      static aarch64-musl binary
#     external.manifest.json               AppLoad manifest
#     icon.png                             launcher icon
#     INSTALL.txt                          on-device install + API-key steps
#   dist/openrouter-chat-appload.tar.gz    the same directory, packed
#
# Prerequisites on the host:
#   - Rust (rustup) with the aarch64-unknown-linux-musl target
#   - Zig 0.13 on PATH (provides the linker for cargo-zigbuild)
#   - cargo-zigbuild
#
# Any extra arguments are forwarded to cargo-zigbuild, e.g.:
#   ./tools/build-appload.sh -j 2
# On memory-constrained hosts, disable fat LTO for the local run:
#   CARGO_PROFILE_RELEASE_LTO=false ./tools/build-appload.sh
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET=aarch64-unknown-linux-musl
BIN="target/${TARGET}/release/openrouter-chat"
OUT=dist/openrouter-chat
TARBALL=dist/openrouter-chat-appload.tar.gz

command -v cargo >/dev/null || { echo "error: cargo not found (install rustup)" >&2; exit 1; }
command -v zig >/dev/null || { echo "error: zig not found on PATH (need Zig 0.13)" >&2; exit 1; }
cargo zigbuild --help >/dev/null 2>&1 || { echo "error: cargo-zigbuild not found (cargo install cargo-zigbuild --locked)" >&2; exit 1; }

echo "==> Building ${TARGET} release binary"
cargo zigbuild --target "$TARGET" --release --locked --bin openrouter-chat "$@"

echo "==> Verifying binary"
file "$BIN"
ls -lh "$BIN"

echo "==> Assembling AppLoad bundle"
rm -rf dist
mkdir -p "$OUT"
cp "$BIN" "$OUT/openrouter-chat"
chmod +x "$OUT/openrouter-chat"
cp external.manifest.json "$OUT/external.manifest.json"
if [ -f assets/icon.png ]; then
    cp assets/icon.png "$OUT/icon.png"
fi

cat > "$OUT/INSTALL.txt" <<'EOF'
OpenRouter Chat — reMarkable Paper Pro
======================================

1. Copy this whole directory to the device:

     scp -r openrouter-chat root@remarkable:/home/root/xovi/exthome/appload/

   Resulting layout on the device:

     /home/root/xovi/exthome/appload/openrouter-chat/openrouter-chat
     /home/root/xovi/exthome/appload/openrouter-chat/external.manifest.json
     /home/root/xovi/exthome/appload/openrouter-chat/icon.png

2. Add your OpenRouter API key (https://openrouter.ai/keys).
   Edit external.manifest.json and add an "environment" block:

     {
       "name": "OpenRouter Chat",
       "application": "openrouter-chat",
       "qtfb": true,
       "environment": { "OPENROUTER_API_KEY": "sk-or-v1-..." }
     }

3. Launch it: XOVI menu -> AppLoad -> "OpenRouter Chat".

The API key is stored as plain text on the device; anyone with root/SSH
access can read it. Without a key the app still opens and the header
shows "No API key". Tap the gear icon for Settings: check the masked
key, run Test API, pick a free model, and adjust text size/colour.
EOF

echo "==> Packing tarball"
tar -czf "$TARBALL" -C dist openrouter-chat

echo "==> Bundle contents"
find dist -type f -maxdepth 2 -exec ls -lh {} \;
echo
echo "Done."
echo "  App directory: $OUT"
echo "  Tarball:       $TARBALL"
