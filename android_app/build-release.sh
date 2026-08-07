#!/bin/bash

# Build and sign release APK for Reminisce app
# Usage: ./build-release.sh

set -e  # Exit on error

# Suppress Java 21+ restricted method warnings
export JAVA_OPTS="$JAVA_OPTS --enable-native-access=ALL-UNNAMED"
export GRADLE_OPTS="$GRADLE_OPTS --enable-native-access=ALL-UNNAMED"

echo "========================================="
echo "Building Reminisce Release APK"
echo "========================================="
echo ""

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Check if keystore.properties exists
if [ ! -f "keystore.properties" ]; then
    echo -e "${RED}Error: keystore.properties file not found${NC}"
    exit 1
fi

# Read properties
KEYSTORE_FILE=$(grep "storeFile=" keystore.properties | cut -d'=' -f2)
# Expand ~ to $HOME if present
KEYSTORE_FILE="${KEYSTORE_FILE/#\~/$HOME}"

# Check if keystore exists
if [ ! -f "$KEYSTORE_FILE" ]; then
    echo -e "${RED}Error: Keystore file not found at $KEYSTORE_FILE${NC}"
    exit 1
fi

# Find apksigner
APKSIGNER=$(find $HOME/Library/Android/sdk/build-tools -name "apksigner" 2>/dev/null | sort -V | tail -1)
if [ -z "$APKSIGNER" ]; then
    echo -e "${RED}Error: apksigner not found in Android SDK${NC}"
    exit 1
fi

echo -e "${BLUE}Step 1/4: Cleaning previous build...${NC}"
./gradlew clean

echo ""
echo -e "${BLUE}Step 2/4: Running unit tests...${NC}"
./gradlew :app:testDebugUnitTest

echo ""
echo -e "${BLUE}Step 3/4: Building release APK...${NC}"
# Gradle signs the release build via `signingConfig signingConfigs.release`
# (keystore.properties) — no separate apksigner step needed.
./gradlew assembleRelease

echo ""
echo -e "${BLUE}Step 4/4: Verifying signature...${NC}"
SIGNED_APK="app/build/outputs/apk/release/app-release.apk"
"$APKSIGNER" verify --verbose "$SIGNED_APK" 2>&1 | grep -E "Verifies|Verified using" | head -5

echo ""
echo -e "${GREEN}=========================================${NC}"
echo -e "${GREEN}Build Complete!${NC}"
echo -e "${GREEN}=========================================${NC}"
echo ""
echo -e "Signed APK: ${GREEN}$SIGNED_APK${NC}"
echo ""
ls -lh "$SIGNED_APK"
echo ""
echo "To install on connected device:"
echo -e "${BLUE}adb install $SIGNED_APK${NC}"
echo ""
