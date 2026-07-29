#!/bin/sh

set -eu

if [ -z "${BATCH_GIT_INSTALL_PATH:-}" ]; then
    echo "error: BATCH_GIT_INSTALL_PATH is not set" >&2
    exit 1
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"

if [ -e "$BATCH_GIT_INSTALL_PATH" ] && [ ! -d "$BATCH_GIT_INSTALL_PATH" ]; then
    echo "error: BATCH_GIT_INSTALL_PATH is not a directory: $BATCH_GIT_INSTALL_PATH" >&2
    exit 1
fi

cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"
mkdir -p "$BATCH_GIT_INSTALL_PATH"
install -m 755 "$TARGET_DIR/release/batch-git" "$BATCH_GIT_INSTALL_PATH/batch-git"

echo "Installed batch-git to $BATCH_GIT_INSTALL_PATH/batch-git"
