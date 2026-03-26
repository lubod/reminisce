#!/bin/sh
# Generate self-signed SSL certificates for the fullstack dev environment.

set -e

CERT_DIR="/etc/nginx/ssl"
CERT_FILE="$CERT_DIR/cert.pem"
KEY_FILE="$CERT_DIR/key.pem"
DAYS_VALID=365

# Skip if certificates already exist (mounted from host)
if [ -f "$CERT_FILE" ] && [ -f "$KEY_FILE" ]; then
    echo "SSL certificates already exist, skipping generation"
    exit 0
fi

echo "Generating self-signed SSL certificate for fullstack dev environment..."

# Build SAN list
SAN="DNS:localhost,DNS:*.localhost,IP:127.0.0.1"

# Also include any EXTRA_SAN_IPS passed via environment
if [ -n "${EXTRA_SAN_IPS:-}" ]; then
    SAN="${SAN},${EXTRA_SAN_IPS}"
    echo "Including extra SANs: ${EXTRA_SAN_IPS}"
fi

echo "SANs: ${SAN}"

openssl req -x509 -nodes -days $DAYS_VALID \
    -newkey rsa:2048 \
    -keyout "$KEY_FILE" \
    -out "$CERT_FILE" \
    -subj "/C=US/ST=State/L=City/O=Reminisce/CN=localhost" \
    -addext "subjectAltName=${SAN}"

chmod 600 "$KEY_FILE"
chmod 644 "$CERT_FILE"

echo "SSL certificates generated successfully"
echo "Valid for $DAYS_VALID days"
