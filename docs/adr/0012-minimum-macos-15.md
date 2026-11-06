# The minimum macOS is 15 Sequoia, the oldest release CI can test

Contract §10 named macOS 14 Sonoma as the minimum for the whole product, and CI ran the
Rust suite on `macos-14` so that the e2e suite and the live ntstat test caught kernel ABI
differences on the oldest supported release. GitHub retired the `macos-14` runner image
on 2026-11-02. No GitHub-hosted runner runs macOS 14 any more, standard or larger, so CI
can no longer test a release the contract promises to support.

The minimum is now macOS 15 Sequoia, the oldest image GitHub hosts. CI runs the Rust suite
on `macos-15` and `macos-26`.

## Considered Options

- **Keep macOS 14 and leave it untested.** No user loses support, but lull parses kernel
  structures whose layout differs between releases (`socket_fdinfo`, ntstat messages), and
  a mismatch there shows up as wrong activity data or a crash, which would then reach
  Sonoma users first.
- **A self-hosted macOS 14 runner.** Keeps the test, but needs a dedicated Mac or VM that
  someone maintains and secures, and GitHub bills a per-minute platform fee for self-hosted
  runners. That is a lot of upkeep for a small project.

## Consequences

- Users still on macOS 14 keep the last release that supports it. Later releases
  declare macOS 15 in the formula, the cask and the app's deployment target, so Homebrew
  refuses to install them on 14 instead of installing a build nobody tested there.
- The Swift package's deployment target becomes `.macOS(.v15)`, so the app can use
  macOS 15 APIs. One example is `onScrollGeometryChange`, which `PanelScroll` works around
  today.
- The minimum follows the oldest macOS image GitHub hosts. When GitHub retires
  `macos-15`, this decision comes up again.
