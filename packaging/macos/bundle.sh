#!/bin/sh
# Builds JustWrite.app next to this script's repository root. Run on a Mac:  packaging/macos/bundle.sh
set -e
cd "$(dirname "$0")/../.."
cargo build --release
APP=target/JustWrite.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/justwrite "$APP/Contents/MacOS/justwrite"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
echo "Built $APP"
