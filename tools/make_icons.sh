#!/bin/sh
# Makes the app icons of every platform from assets/logo/1-open-book.svg. Run on a Mac (sips,
# iconutil) with Pillow for Python:  tools/make_icons.sh
set -e
cd "$(dirname "$0")/.."
SVG=assets/logo/1-open-book.svg
T=$(mktemp -d)
sips -s format png -z 1024 1024 "$SVG" --out "$T/tile.png" >/dev/null
# iOS rounds the corners itself and takes no transparency.
sed 's/rx="112"//' "$SVG" > "$T/square.svg"
sips -s format png -z 1024 1024 "$T/square.svg" --out "$T/square.png" >/dev/null
IOS=ios/Sources/Assets.xcassets/AppIcon.appiconset
mkdir -p "$IOS" "$T/JustWrite.iconset"
python3 - "$T" "$IOS" <<'PY'
import sys
from PIL import Image
t, ios = sys.argv[1:]
tile = Image.open(f"{t}/tile.png")
Image.open(f"{t}/square.png").convert("RGB").save(f"{ios}/AppIcon.png")
# A macOS icon leaves a margin: the tile is 824 of 1024.
mac = Image.new("RGBA", (1024, 1024))
mac.paste(tile.resize((824, 824), Image.LANCZOS), (100, 100))
for size in (16, 32, 128, 256, 512):
    mac.resize((size, size), Image.LANCZOS).save(f"{t}/JustWrite.iconset/icon_{size}x{size}.png")
    mac.resize((size * 2, size * 2), Image.LANCZOS).save(f"{t}/JustWrite.iconset/icon_{size}x{size}@2x.png")
tile.save("packaging/windows/justwrite.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 256)])
tile.resize((256, 256), Image.LANCZOS).save("packaging/linux/justwrite.png")
PY
iconutil -c icns "$T/JustWrite.iconset" -o packaging/macos/JustWrite.icns
rm -rf "$T"
