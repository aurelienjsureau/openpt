#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/macos"
VERSION="${OPENPT_VERSION:-$(awk '/^\[/{p=($0=="[workspace.package]");next} p&&$1=="version"{gsub(/[" ]/,"",$3);print $3;exit}' "$ROOT/Cargo.toml")}"
LABEL="${OPENPT_LABEL:-$VERSION}"
SHORT_VERSION="${VERSION%%-*}"
SHA="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
TARGET=aarch64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=11.0
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIST="${DIST:-$ROOT/dist}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/OpenPT.app"
DMG="$DIST/OpenPT-$LABEL-macos-arm64.dmg"

if [ "${SKIP_BUILD:-0}" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p openpt --target "$TARGET")
fi

rm -rf "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DIST"
cp "$CARGO_TARGET_DIR/$TARGET/release/openpt" "$APP/Contents/MacOS/OpenPT"
cp "$ROOT/assets/app-icon/openpt.icns" "$APP/Contents/Resources/OpenPT.icns"
sed -e "s/@VERSION@/$LABEL/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" -e "s/@BUILD_SHA@/$SHA/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"
for f in LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md; do
  [ -f "$ROOT/$f" ] && cp "$ROOT/$f" "$APP/Contents/Resources/"
done
xattr -cr "$APP"
codesign --force --sign - --timestamp=none --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/OpenPT"
codesign --force --sign - --timestamp=none --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"
otool -L "$APP/Contents/MacOS/OpenPT"

STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/OpenPT.app"
ln -s /Applications "$STAGE/Applications"
cat >"$STAGE/LISEZ-MOI.txt" <<TXT
OpenPT $LABEL (alpha) pour Mac Apple Silicon

1. Glisser OpenPT.app dans Applications.
2. Application non signée par Apple : au premier lancement, clic droit sur OpenPT > Ouvrir > Ouvrir.
   Si macOS dit que l'application est endommagée, dans le Terminal :
       xattr -cr /Applications/OpenPT.app
3. Setup > Playback Engine : choisir l'interface audio (CoreAudio).

ARA (Melodyne) est activé sur Mac mais pas encore testé dans cette alpha.
TXT
rm -f "$DMG"
for attempt in 1 2 3 4 5; do
  hdiutil create -srcfolder "$STAGE" -volname "OpenPT" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG" && break
  [ "$attempt" = 5 ] && exit 1
  sleep $((attempt * 10))
done
ls -lh "$DMG"
