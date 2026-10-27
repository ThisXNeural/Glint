#!/bin/bash
# Signs one Windows executable in place with the certificate in SIGN_PFX
# (password in SIGN_PASSWORD). Adds an RFC 3161 timestamp when the
# timestamp server is reachable, so the signature outlives the certificate.
set -e
TIMESTAMP_URL=${TIMESTAMP_URL:-http://timestamp.digicert.com}
args=(-pkcs12 "$SIGN_PFX" -pass "$SIGN_PASSWORD" -n "Glint"
      -i "https://github.com/ThisXNeural/Glint" -h sha256)
rm -f "$1.signed"
if ! osslsigncode sign "${args[@]}" -ts "$TIMESTAMP_URL" -in "$1" -out "$1.signed" >/dev/null 2>&1; then
  echo "timestamp server unreachable, signing without a timestamp" >&2
  rm -f "$1.signed"
  osslsigncode sign "${args[@]}" -in "$1" -out "$1.signed" >/dev/null
fi
mv "$1.signed" "$1"
echo "signed $(basename "$1")"
