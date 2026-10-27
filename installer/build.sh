#!/bin/bash
# Builds Glint.exe and GlintSetup.exe into installer/out.
# Cross-compiles from Linux with mingw (the same way CI does).
#
# Optional code signing: set SIGN_PFX to a .pfx certificate file and
# SIGN_PASSWORD to its password. Both executables are then signed with
# osslsigncode and timestamped. Without them the build is unsigned.
set -e
cd "$(dirname "$0")/.."
TARGET=${TARGET:-x86_64-pc-windows-gnu}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/glint/Cargo.toml | head -1)
SIGN_SCRIPT="$PWD/installer/sign.sh"
NSIS_SIGN=()

sign() {
  [ -n "$SIGN_PFX" ] || return 0
  "$SIGN_SCRIPT" "$1"
}
if [ -n "$SIGN_PFX" ]; then
  NSIS_SIGN=(-DSIGN="$SIGN_SCRIPT")
fi

cargo build --release -p glint --target "$TARGET"
mkdir -p installer/out
cp "${CARGO_TARGET_DIR:-target}/$TARGET/release/glint.exe" installer/out/Glint.exe
sign installer/out/Glint.exe

cp installer/out/Glint.exe installer/Glint.exe
cp crates/glint/assets/glint.ico installer/glint.ico
(cd installer && makensis -V2 -DVERSION="$VERSION" "${NSIS_SIGN[@]}" glint.nsi && mv GlintSetup.exe out/)
rm installer/Glint.exe installer/glint.ico

(cd installer/out && sha256sum Glint.exe GlintSetup.exe > SHA256SUMS.txt)
ls -la installer/out
