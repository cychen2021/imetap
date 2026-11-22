#!/bin/bash
# Installs imetap on macOS and signs it with a stable self-signed certificate.
#
# An ad-hoc signed binary is identified by its hash, so every `cargo install` looks like a new
# app to macOS and loses the Accessibility permission. Signed with a fixed certificate and
# identifier, the binary keeps the permission across reinstalls.
#
# Usage: scripts/install-mac.sh [--no-install]
#   --no-install  only sign the binary that is already installed
# Environment: IMETAP_CERT names the certificate (default "imetap Code Signing").
set -euo pipefail

CERT="${IMETAP_CERT:-imetap Code Signing}"
IDENTIFIER="io.github.cychen2021.imetap"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "$(uname)" != Darwin ]]; then
    echo "This script is for macOS only." >&2
    exit 1
fi

install=1
for arg in "$@"; do
    case "$arg" in
        --no-install) install=0 ;;
        -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

create_cert() {
    echo "Creating the self-signed certificate \"$CERT\" in the login keychain..."
    local dir
    dir="$(mktemp -d)"
    trap 'rm -rf "$dir"' RETURN
    cat > "$dir/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $CERT
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
    # The system LibreSSL writes a PKCS#12 file that `security import` can read. OpenSSL 3
    # would need `-legacy`.
    /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
        -config "$dir/cert.cnf" -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
    /usr/bin/openssl pkcs12 -export -inkey "$dir/key.pem" -in "$dir/cert.pem" \
        -out "$dir/cert.p12" -passout pass:imetap
    # -T lets codesign use the key without asking each time.
    security import "$dir/cert.p12" -k "$KEYCHAIN" -P imetap -T /usr/bin/codesign >/dev/null
}

if ! security find-certificate -c "$CERT" "$KEYCHAIN" >/dev/null 2>&1; then
    create_cert
    first_cert=1
else
    first_cert=0
fi

if [[ $install == 1 ]]; then
    cargo install --path "$ROOT"
fi

BIN="$(command -v imetap || true)"
BIN="${BIN:-$HOME/.cargo/bin/imetap}"
if [[ ! -x "$BIN" ]]; then
    echo "imetap is not installed at $BIN." >&2
    exit 1
fi

echo "Signing $BIN..."
codesign --force --sign "$CERT" --identifier "$IDENTIFIER" "$BIN"
codesign --display --requirements - "$BIN" 2>&1 | grep designated

if pgrep -qx imetap; then
    "$BIN" restart
fi

if [[ $first_cert == 1 ]]; then
    cat <<EOF

One-time step (only now, because the certificate was just created): the Accessibility entry
granted to the old ad-hoc signed imetap no longer matches. In System Settings > Privacy &
Security > Accessibility, remove the old imetap entry with "-", then grant access when imetap
asks (or add $BIN with "+").

You will not need to do this again: later runs of this script reuse the same certificate, so
the permission survives reinstalls.
EOF
fi
