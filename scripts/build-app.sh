#!/bin/sh
# Builds macos/.build/Lull.app and signs it ad hoc (M4 publishes nothing; Developer ID
# signing and notarisation come with M5). Usage: scripts/build-app.sh [release|debug]
set -eu
config=${1:-release}
cd "$(dirname "$0")/../macos"
swift build -c "$config" --product LullApp
app=.build/Lull.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$(swift build -c "$config" --show-bin-path)/LullApp" "$app/Contents/MacOS/Lull"
# The icon is drawn by the app from the logo's pixel grid; no image is committed.
iconset=$(mktemp -d)/AppIcon.iconset
"$app/Contents/MacOS/Lull" --write-iconset "$iconset"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' ../Cargo.toml | head -n 1)
sed "s/@VERSION@/$version/g" Resources/Info.plist >"$app/Contents/Info.plist"
codesign --force --sign - "$app"
echo "built $(pwd)/$app"
