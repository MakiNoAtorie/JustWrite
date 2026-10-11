#!/bin/sh
# Builds the Rust library for the platform Xcode is building. Run by the Xcode project.
set -e
export PATH="$HOME/.cargo/bin:$PATH"
case "$PLATFORM_NAME" in
  iphonesimulator) target=aarch64-apple-ios-sim; [ "$ARCHS" = x86_64 ] && target=x86_64-apple-ios ;;
  *) target=aarch64-apple-ios ;;
esac
# Xcode points SDKROOT at the iOS SDK; build scripts of crates run on the Mac and must not see it.
unset SDKROOT
cd "$SRCROOT/.."
cargo rustc --lib --release --target "$target" --crate-type staticlib
mkdir -p "$SRCROOT/build-rust"
cp "target/$target/release/libjustwrite.a" "$SRCROOT/build-rust/"
