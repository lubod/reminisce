#!/bin/bash
set -e

# Configuration
NDK_VERSION="26.3.11579264"
ANDROID_PROJECT_DIR="android_app"
RUST_PROJECT_DIR="np2p-mobile"
PACKAGE_NAME="com.reminisce.p2p"

# Ensure correct PATH for rustup and cargo tools
export PATH="/Users/ldr/.cargo/bin:$PATH"

# Set Android SDK and NDK paths
export ANDROID_HOME="$HOME/Library/Android/sdk"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$NDK_VERSION"

# 1. Install Android targets if missing
echo "Checking Rust targets for Android..."
rustup target add aarch64-linux-android x86_64-linux-android

# 2. Build for host to facilitate binding generation
echo "Building for host..."
cargo build --release -p $RUST_PROJECT_DIR

# 3. Build uniffi-bindgen tool
echo "Building uniffi-bindgen..."
cargo build -p $RUST_PROJECT_DIR --bin uniffi-bindgen

# 4. Generate Kotlin bindings
echo "Generating Kotlin bindings..."
mkdir -p $ANDROID_PROJECT_DIR/app/src/main/java
# Determine library extension based on OS
LIB_EXT="so"
if [[ "$OSTYPE" == "darwin"* ]]; then
    LIB_EXT="dylib"
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    LIB_EXT="dll"
fi

# Use the host-built library for binding generation
./target/debug/uniffi-bindgen generate --library target/release/libnp2p_mobile.$LIB_EXT --language kotlin --out-dir $ANDROID_PROJECT_DIR/app/src/main/java

# 5. Build Rust libraries for Android
echo "Building Rust libraries for Android..."
cargo ndk -t aarch64-linux-android -t x86_64-linux-android -P 24 build --release --manifest-path $RUST_PROJECT_DIR/Cargo.toml

# 6. Copy native libraries to JNI directories
echo "Copying .so files to Android project..."
JNI_DIR="$ANDROID_PROJECT_DIR/app/src/main/jniLibs"
mkdir -p $JNI_DIR/arm64-v8a
mkdir -p $JNI_DIR/x86_64

cp target/aarch64-linux-android/release/libnp2p_mobile.so $JNI_DIR/arm64-v8a/
cp target/x86_64-linux-android/release/libnp2p_mobile.so $JNI_DIR/x86_64/

echo "✅ Build complete! Native libraries and Kotlin bindings are ready."
