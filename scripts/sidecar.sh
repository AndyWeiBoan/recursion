#!/usr/bin/env bash
# Build recursiond and place it where Tauri's externalBin expects it (name suffixed with the target triple).
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${1:-debug}"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
if [ "$PROFILE" = "release" ]; then cargo build -p recursiond --release; else cargo build -p recursiond; fi
mkdir -p src-tauri/binaries
cp "target/$PROFILE/recursiond" "src-tauri/binaries/recursiond-$TRIPLE"
