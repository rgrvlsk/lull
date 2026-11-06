#!/bin/sh
# Publishes a lull release whose version bump is already merged to main (packaging/RELEASE.md):
# tags origin/main, builds the extension zip from the tag, creates the GitHub release and
# updates Formula/lull.rb in rgrvlsk/homebrew-tap. The maintainer runs it; .claude/settings.json
# keeps agents from pushing to main and creating releases.
# Usage: scripts/release.sh <version> <notes.md>
set -eu
[ $# -eq 2 ] || { echo "usage: $0 <version> <notes.md>" >&2; exit 2; }
V=$1
notes=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
cd "$(dirname "$0")/.."
tap=$(brew --repository rgrvlsk/tap)
zip=lull-extension-chromium-$V.zip

fail() { echo "release: $*" >&2; exit 1; }
git fetch -q origin main
for f in Cargo.toml extension/package.json extension/manifests/chromium.json; do
  git show "origin/main:$f" | grep -Eq "^ *\"?version\"? ?[=:] \"$V\"" || fail "$f on origin/main is not $V"
done
git -C "$tap" pull -q --ff-only
[ -z "$(git -C "$tap" status --porcelain)" ] || fail "$tap has uncommitted changes"
if git rev-parse -q --verify "refs/tags/v$V" >/dev/null; then
  [ "$(git rev-parse "v$V^{commit}")" = "$(git rev-parse origin/main)" ] || fail "v$V exists and is not origin/main"
else
  git tag "v$V" origin/main
fi

work=$(mktemp -d)
trap 'git worktree remove --force "$work/src" 2>/dev/null; rm -rf "$work"' EXIT
git worktree add -q --detach "$work/src" "v$V"
(cd "$work/src/extension" && npm ci --silent && npm run -s build && cd dist/chromium && zip -qrX "$work/$zip" .)
ext_sha=$(shasum -a 256 "$work/$zip" | cut -d' ' -f1)

git push origin "v$V"
gh release create "v$V" "$work/$zip" --title "v$V" --notes-file "$notes"

src_sha=$(curl -fsSL "https://github.com/rgrvlsk/lull/archive/refs/tags/v$V.tar.gz" | shasum -a 256 | cut -d' ' -f1)
dl_sha=$(curl -fsSL "https://github.com/rgrvlsk/lull/releases/download/v$V/$zip" | shasum -a 256 | cut -d' ' -f1)
[ "$dl_sha" = "$ext_sha" ] || fail "uploaded $zip has sha256 $dl_sha, built $ext_sha"

grep -v '^# ' packaging/homebrew/lull.rb |
  sed -e "s/@VERSION@/$V/g" -e "s/@SOURCE_SHA256@/$src_sha/" -e "s/@EXTENSION_SHA256@/$ext_sha/" \
  >"$tap/Formula/lull.rb"
brew audit --strict --online rgrvlsk/tap/lull
git -C "$tap" commit -q -a -m "lull $V"
git -C "$tap" push -q origin main
echo "released v$V: source sha256 $src_sha, extension sha256 $ext_sha"
