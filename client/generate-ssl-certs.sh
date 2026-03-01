#!/bin/sh
# Generate self-signed SSL certificates for development/testing
# This script runs automatically in the Docker entrypoint
# For production, mount real certificates instead

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

echo "Generating self-signed SSL certificate for development..."
echo "Valid for $DAYS_VALID days"

# Generate private key and certificate
openssl req -x509 -nodes -days $DAYS_VALID \
    -newkey rsa:2048 \
    -keyout "$KEY_FILE" \
    -out "$CERT_FILE" \
    -subj "/C=US/ST=State/L=City/O=Reminisce/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,DNS:*.localhost,IP:127.0.0.1"

# Set proper permissions
chmod 600 "$KEY_FILE"
chmod 644 "$CERT_FILE"

echo "✓ SSL certificates generated successfully"
echo "NOTE: Self-signed certificate - browsers will show warnings"
