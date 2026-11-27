#!/usr/bin/env bash
# Publish a new release (macOS, and Linux when run on Linux) that installed apps update to by themselves.
#   bash scripts/release.sh              # next patch version (0.1.3 -> 0.1.4)
#   bash scripts/release.sh 0.2.0        # a given version
#   bash scripts/release.sh --dry-run    # build everything, upload and commit nothing
#
# On a Mac it builds right here (macOS only). On Linux it builds the Linux packages (AppImage,
# rpm, deb; this machine's architecture) here and the macOS one on the Mac mini (ssh macmini,
# where the repo is synced to ~/src/recursion). Either way the x86_64 Linux packages are built
# on GitHub meanwhile (.github/workflows/linux-x86_64.yml), from the commit pushed there. The update is signed with
# ~/.tauri/recursion-updater.key (on the machine that builds) and uploaded to the public
# AndyWeiBoan/recursion-releases repo, whose latest.json the app checks
# (tauri.conf.json > plugins.updater). Then the version bump is committed and pushed.
#
# Needs: python3, node (pnpm or npx), cargo with tauri-cli (`cargo tauri` or `npx tauri`),
# and gh logged in to GitHub on the machine running this script.
set -euo pipefail
cd "$(dirname "$0")/.."
MAC=macmini
REPO=AndyWeiBoan/recursion-releases
SRC_REPO=AndyWeiBoan/recursion
OUT=target/release-upload
KEY=~/.tauri/recursion-updater.key
CONFIG='{"build":{"beforeBuildCommand":"bash scripts/sidecar.sh release"},"bundle":{"macOS":{"signingIdentity":"-"}}}'
# Without this, big copies over ssh to the Mac stall after a few hundred KB on some networks.
export RSYNC_RSH="ssh -o IPQoS=none"

dry=false
ver=
for a in "$@"; do
  case "$a" in
    --dry-run) dry=true ;;
    -*) echo "unknown option $a" >&2; exit 2 ;;
    *) ver=$a ;;
  esac
done
local_build=false
[ "$(uname)" = Darwin ] && local_build=true
linux=false
[ "$(uname)" = Linux ] && linux=true

die() { echo "release: $*" >&2; exit 1; }
# `timeout` is GNU; macOS has it only as gtimeout (coreutils), if at all.
to() {
  local s=$1; shift
  if command -v timeout >/dev/null; then timeout "$s" "$@"
  elif command -v gtimeout >/dev/null; then gtimeout "$s" "$@"
  else "$@"; fi
}

# Check everything before changing anything.
command -v python3 >/dev/null || die "needs python3"
command -v node >/dev/null || die "needs Node (for the UI build)"
$dry || command -v gh >/dev/null || die "needs gh (brew install gh, then gh auth login)"
$dry || gh auth status >/dev/null 2>&1 || die "gh is not logged in (gh auth login)"
if $local_build || $linux; then
  [ -f "$KEY" ] || die "no signing key at $KEY (copy it from the machine that has it)"
fi
if ! $local_build; then
  to 20 ssh "$MAC" true || die "can't reach $MAC over ssh"
fi
# What gets published must be in git: no uncommitted changes (agents' design/ and docs/ drafts aside).
if ! $dry && [ -n "$(git status --porcelain --untracked-files=no -- . ':!design' ':!docs')" ]; then
  git status --short --untracked-files=no -- . ':!design' ':!docs' >&2
  die "commit (and push) these first"
fi

# GitHub builds x86_64 from what it has: this very commit must be pushed.
sha=$(git rev-parse HEAD)
x86=true
if ! git fetch -q origin || [ -z "$(git branch -r --contains "$sha")" ]; then
  $dry || die "push this commit first (GitHub builds the x86_64 packages from it)"
  echo "== not pushed: the dry run skips the x86_64 build"
  x86=false
fi

cur=$(python3 -c 'import re; print(re.search(r"^version = \"(.*)\"", open("Cargo.toml").read(), re.M).group(1))')
[ -n "$ver" ] || ver=$(echo "$cur" | awk -F. '{print $1"."$2"."$3+1}')
echo "== $cur -> $ver$($dry && echo ' (dry run)')"

python3 scripts/set-version.py "$ver"
cargo metadata --format-version 1 >/dev/null # refresh Cargo.lock
VERSION_FILES=(Cargo.toml Cargo.lock package.json src-tauri/tauri.conf.json)
# A dry run leaves the version as it was.
$dry && trap 'git checkout -q -- "${VERSION_FILES[@]}"' EXIT

if command -v pnpm >/dev/null; then pnpm exec vite build; else npx vite build; fi

rm -rf "$OUT" && mkdir -p "$OUT"

# Start the x86_64 build on GitHub; it runs while we build here, and is collected at the end.
run=
if $x86; then
  title="x86_64 $ver $sha"
  # Runs with this title from before (a retried release) are not ours: wait for a new one.
  runs() { gh run list --repo "$SRC_REPO" --workflow linux-x86_64.yml --limit 20 --json databaseId,displayTitle \
    -q ".[] | select(.displayTitle == \"$title\") | .databaseId"; }
  old=" $(runs | tr '\n' ' ') "
  gh workflow run linux-x86_64.yml --repo "$SRC_REPO" -f ref="$sha" -f version="$ver"
  for _ in $(seq 30); do
    for id in $(runs); do case "$old" in *" $id "*) ;; *) run=$id; break ;; esac; done
    [ -n "$run" ] && break
    sleep 2
  done
  [ -n "$run" ] || die "the x86_64 build on GitHub didn't start"
  echo "== x86_64 building on GitHub: https://github.com/$SRC_REPO/actions/runs/$run"
fi
if cargo tauri --version >/dev/null 2>&1; then tauri=(cargo tauri); else tauri=(npx tauri); fi

# Linux first: it's quick, and a failure here stops us before the long Mac build.
ARCH=$(uname -m)
DEB_ARCH=$([ "$ARCH" = aarch64 ] && echo arm64 || echo amd64)
LINUX_FILES=("appimage/Recursion_${ver}_${ARCH}.AppImage" "rpm/Recursion-${ver}-1.${ARCH}.rpm" "deb/Recursion_${ver}_${DEB_ARCH}.deb")
if $linux; then
  CI=true TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD= \
    "${tauri[@]}" build --bundles appimage,rpm,deb
  for f in "${LINUX_FILES[@]}"; do cp "target/release/bundle/$f" "target/release/bundle/$f.sig" "$OUT/"; done
fi

FILES=(macos/Recursion.app.tar.gz macos/Recursion.app.tar.gz.sig "dmg/Recursion_${ver}_aarch64.dmg")
if $local_build; then
  CI=true TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD= \
    "${tauri[@]}" build --bundles app,dmg --config "$CONFIG"
  B=target/release/bundle
  for f in "${FILES[@]}"; do cp "$B/$f" "$OUT/"; done
  # A second Recursion.app with the same bundle id makes the Dock show extra Recursion icons.
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$B/macos/Recursion.app" || true
  rm -rf "$B/macos/Recursion.app"
else
  for try in 1 2 3 4 5; do
    to 300 rsync -a --delete --timeout=30 --exclude target --exclude node_modules --exclude .git --exclude src-tauri/binaries ./ "$MAC:src/recursion/" && break
    [ "$try" = 5 ] && die "could not sync to $MAC"
    sleep 5
  done
  ssh "$MAC" zsh -lc "'cd ~/src/recursion && CI=true TAURI_SIGNING_PRIVATE_KEY=\$(cat ~/.tauri/recursion-updater.key) TAURI_SIGNING_PRIVATE_KEY_PASSWORD= cargo tauri build --bundles app,dmg --config '\''$CONFIG'\'''"
  B=src/recursion/target/release/bundle
  # rsync resumes; the link to the Mac sometimes stalls mid-copy.
  for f in "${FILES[@]}"; do
    for try in 1 2 3 4 5; do
      to 180 rsync -a --partial --timeout=30 "$MAC:$B/$f" "$OUT/" && break
      [ "$try" = 5 ] && die "could not copy $f"
    done
  done
  ssh "$MAC" "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u ~/$B/macos/Recursion.app; rm -rf ~/$B/macos/Recursion.app"
fi

X86_FILES=()
if [ -n "$run" ]; then
  gh run watch "$run" --repo "$SRC_REPO" --exit-status --interval 15 >/dev/null || die "the x86_64 build failed: https://github.com/$SRC_REPO/actions/runs/$run"
  gh run download "$run" --repo "$SRC_REPO" -n linux-x86_64 -D "$OUT"
  for f in "$OUT"/*; do
    case "$f" in *.sig) ;; *x86_64*|*amd64*) X86_FILES+=("${f##*/}") ;; esac
  done
  [ "${#X86_FILES[@]}" = 3 ] || die "expected 3 x86_64 packages, got: ${X86_FILES[*]}"
fi
# (the ${a[@]+...} form: macOS's bash 3.2 calls an empty array unbound under set -u)
ALL_LINUX=($($linux && printf '%s ' "${LINUX_FILES[@]##*/}") ${X86_FILES[@]+"${X86_FILES[@]}"})

url="https://github.com/$REPO/releases/download/v$ver/Recursion.app.tar.gz"
python3 - "$ver" "$url" "$OUT" ${ALL_LINUX[@]+"${ALL_LINUX[@]}"} <<'EOF'
import json, sys, datetime
ver, url, out, *linux = sys.argv[1:]
sig = lambda f: open(f"{out}/{f}.sig").read().strip()
platforms = {"darwin-aarch64": {"signature": sig("Recursion.app.tar.gz"), "url": url}}
# The updater looks for <os>-<arch>-<how it was installed>, then <os>-<arch>.
for f in linux:
    arch = "x86_64" if ("x86_64" in f or "amd64" in f) else "aarch64"
    kind = f.rsplit(".", 1)[1].lower()
    entry = {"signature": sig(f), "url": url.rsplit("/", 1)[0] + "/" + f}
    platforms[f"linux-{arch}-{kind}"] = entry
    if kind == "appimage":
        platforms[f"linux-{arch}"] = entry
json.dump({"version": ver, "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
           "platforms": platforms}, open(f"{out}/latest.json", "w"), indent=2)
EOF

if $dry; then
  ls -l "$OUT"
  echo "== dry run: built v$ver in $OUT; nothing uploaded or committed"
  exit 0
fi

gh release create "v$ver" --repo "$REPO" --title "Recursion $ver" --notes "" \
  "$OUT/latest.json" "$OUT/Recursion.app.tar.gz" "$OUT/Recursion_${ver}_aarch64.dmg" \
  $(for f in ${ALL_LINUX[@]+"${ALL_LINUX[@]}"}; do printf '%s ' "$OUT/$f"; done)
echo "== published v$ver: https://github.com/$REPO/releases/tag/v$ver"

git add "${VERSION_FILES[@]}"
git commit -q -m "Release $ver"
git push -q
echo "== committed and pushed: Release $ver"
