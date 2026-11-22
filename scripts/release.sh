#!/usr/bin/env bash
# Publish a new macOS release that installed apps update to by themselves.
#   bash scripts/release.sh            # next patch version (0.1.3 -> 0.1.4)
#   bash scripts/release.sh 0.2.0      # a given version
# Builds on the Mac mini (ssh macmini), signs the update with ~/.tauri/recursion-updater.key
# (also copied to the Mac), and uploads it to the public AndyWeiBoan/recursion-releases repo,
# whose latest.json the app checks (tauri.conf.json > plugins.updater).
set -euo pipefail
cd "$(dirname "$0")/.."
MAC=macmini
# Without this, big copies over ssh to the Mac stall after a few hundred KB on this network.
export RSYNC_RSH="ssh -o IPQoS=none"
REPO=AndyWeiBoan/recursion-releases
OUT=target/release-upload

cur=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if [ $# -ge 1 ]; then ver=$1; else ver=$(echo "$cur" | awk -F. '{print $1"."$2"."$3+1}'); fi
echo "== $cur -> $ver"

# One version everywhere: the app compares tauri.conf.json's with latest.json's.
sed -i "0,/^version = \".*\"/s//version = \"$ver\"/" Cargo.toml
for f in src-tauri/tauri.conf.json package.json; do
  sed -i "0,/\"version\": \".*\"/s//\"version\": \"$ver\"/" "$f"
done
cargo metadata --format-version 1 >/dev/null # refresh Cargo.lock

npx vite build
rsync -a --delete --exclude target --exclude node_modules --exclude .git --exclude src-tauri/binaries ./ "$MAC:src/recursion/"
ssh "$MAC" zsh -lc "'cd ~/src/recursion && CI=true TAURI_SIGNING_PRIVATE_KEY=\$(cat ~/.tauri/recursion-updater.key) TAURI_SIGNING_PRIVATE_KEY_PASSWORD= \
  cargo tauri build --bundles app,dmg --config '\''{\"build\":{\"beforeBuildCommand\":\"bash scripts/sidecar.sh release\"},\"bundle\":{\"macOS\":{\"signingIdentity\":\"-\"}}}'\'''"

rm -rf "$OUT" && mkdir -p "$OUT"
B=src/recursion/target/release/bundle
# rsync resumes; the link to the Mac sometimes stalls mid-copy.
for f in macos/Recursion.app.tar.gz macos/Recursion.app.tar.gz.sig "dmg/Recursion_${ver}_aarch64.dmg"; do
  for try in 1 2 3 4 5; do
    timeout 180 rsync -a --partial --timeout=30 "$MAC:$B/$f" "$OUT/" && break
    [ "$try" = 5 ] && { echo "could not copy $f" >&2; exit 1; }
  done
done
# A second Recursion.app on the Mac (same bundle id) makes its Dock show extra Recursion icons.
ssh "$MAC" "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u ~/$B/macos/Recursion.app; rm -rf ~/$B/macos/Recursion.app"

url="https://github.com/$REPO/releases/download/v$ver/Recursion.app.tar.gz"
python3 - "$ver" "$url" "$OUT" <<'EOF'
import json, sys, datetime
ver, url, out = sys.argv[1:]
sig = open(f"{out}/Recursion.app.tar.gz.sig").read().strip()
json.dump({"version": ver, "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
           "platforms": {"darwin-aarch64": {"signature": sig, "url": url}}}, open(f"{out}/latest.json", "w"), indent=2)
EOF

gh release create "v$ver" --repo "$REPO" --title "Recursion $ver" --notes "" \
  "$OUT/latest.json" "$OUT/Recursion.app.tar.gz" "$OUT/Recursion_${ver}_aarch64.dmg"
echo "== published v$ver: https://github.com/$REPO/releases/tag/v$ver"
