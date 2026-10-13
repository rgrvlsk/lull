#!/bin/sh
# Builds macos/.build/Lull.app and signs it ad hoc (M4 publishes nothing; Developer ID
# signing and notarisation come with M5). Usage: scripts/build-app.sh [release|debug]
set -eu
config=${1:-release}
cd "$(dirname "$0")/../macos"
swift build -c "$config" --product LullApp
app=.build/Lull.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$(swift build -c "$config" --show-bin-path)/LullApp" "$app/Contents/MacOS/Lull"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' ../Cargo.toml | head -n 1)
sed "s/@VERSION@/$version/g" Resources/Info.plist >"$app/Contents/Info.plist"
codesign --force --sign - "$app"
echo "built $(pwd)/$app"
