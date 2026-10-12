# Releasing lull (v1: Homebrew formula, ADR-0008)

1. Bump `version` in `Cargo.toml`, `extension/package.json` and `extension/manifests/chromium.json`;
   commit; run the full checks (README "Develop").
2. Tag and push: `git tag v<V> && git push origin main v<V>`.
3. Build the extension asset from the tag:
   `cd extension && npm ci && npm run build && (cd dist/chromium && zip -qrX ../../../lull-extension-chromium-<V>.zip .)`
4. `gh release create v<V> lull-extension-chromium-<V>.zip --title v<V> --notes …`
5. sha256 of the source tarball
   (`curl -sL https://github.com/rgrvlsk/lull/archive/refs/tags/v<V>.tar.gz | shasum -a 256`)
   and of the zip.
6. Copy `packaging/homebrew/lull.rb` to `Formula/lull.rb` in `rgrvlsk/homebrew-tap`, replacing
   `@VERSION@`, `@SOURCE_SHA256@` and `@EXTENSION_SHA256@`; `brew audit --strict --online
   rgrvlsk/tap/lull`; commit and push the tap.

The Chromium extension id is pinned by the public `key` in the manifest. Its private half is
a maintainer secret (ADR-0007), kept outside the repository; it is only needed to publish the
extension to the Chrome Web Store under the same id.
