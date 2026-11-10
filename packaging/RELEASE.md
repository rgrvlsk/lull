# Releasing lull (v1: Homebrew formula, ADR-0008)

1. Bump `version` in `Cargo.toml`, `extension/package.json` and `extension/manifests/chromium.json`;
   commit on a branch; run the full checks (README "Develop").
2. Open the bump as a pull request; the maintainer approves and merges it.
3. Write the release notes to a file and, as the maintainer, run
   `scripts/release.sh <V> <notes.md>`. It tags `origin/main` as `v<V>`, builds
   `lull-extension-chromium-<V>.zip` from the tag, pushes the tag, creates the GitHub release
   with the zip, checks the uploaded zip's sha256, writes `Formula/lull.rb` in
   `rgrvlsk/homebrew-tap` from `packaging/homebrew/lull.rb` with the version and both sha256
   values, runs `brew audit --strict --online rgrvlsk/tap/lull`, and commits and pushes the tap.
   Agents cannot run it: `.claude/settings.json` denies them pushing to `main` and creating
   releases.

The Chromium extension id is pinned by the public `key` in the manifest. Its private half is
a maintainer secret (ADR-0007), kept outside the repository; it is only needed to publish the
extension to the Chrome Web Store under the same id.
