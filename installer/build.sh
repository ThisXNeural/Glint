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
TIMESTAMP_URL=${TIMESTAMP_URL:-http://timestamp.digicert.com}

sign() {
  [ -n "$SIGN_PFX" ] || return 0
  osslsigncode sign -pkcs12 "$SIGN_PFX" -pass "$SIGN_PASSWORD" \
    -n "Glint" -i "https://github.com/ThisXNeural/Glint" \
    -h sha256 -ts "$TIMESTAMP_URL" -in "$1" -out "$1.signed"
  mv "$1.signed" "$1"
  echo "signed $1"
}

cargo build --release -p glint --target "$TARGET"
mkdir -p installer/out
cp "${CARGO_TARGET_DIR:-target}/$TARGET/release/glint.exe" installer/out/Glint.exe
sign installer/out/Glint.exe

cp installer/out/Glint.exe installer/Glint.exe
cp crates/glint/assets/glint.ico installer/glint.ico
(cd installer && makensis -V2 -DVERSION="$VERSION" glint.nsi && mv GlintSetup.exe out/)
rm installer/Glint.exe installer/glint.ico
sign installer/out/GlintSetup.exe

(cd installer/out && sha256sum Glint.exe GlintSetup.exe > SHA256SUMS.txt)
ls -la installer/out
