#!/usr/bin/env bash
set -euo pipefail

echo "🌱 Compilazione release di DiskGarden (v0.1.0-alpha.1)..."
cargo build --release

BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
META_DIR="${HOME}/.local/share/metainfo"

mkdir -p "${BIN_DIR}" "${APP_DIR}" "${META_DIR}"

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
if [[ -z "${TARGET_DIR}" ]]; then
  TARGET_DIR="target"
fi

install -m 0755 "${TARGET_DIR}/release/disk-garden" "${BIN_DIR}/disk-garden"
install -m 0755 "${TARGET_DIR}/release/disk-garden-cli" "${BIN_DIR}/disk-garden-cli"
install -m 0644 packaging/io.github.pietrolama.DiskGarden.desktop "${APP_DIR}/io.github.pietrolama.DiskGarden.desktop"
install -m 0644 packaging/io.github.pietrolama.DiskGarden.metainfo.xml "${META_DIR}/io.github.pietrolama.DiskGarden.metainfo.xml"

echo "✅ DiskGarden installato con successo in:"
echo "   - ${BIN_DIR}/disk-garden"
echo "   - ${BIN_DIR}/disk-garden-cli"
echo "   - ${APP_DIR}/io.github.pietrolama.DiskGarden.desktop"
