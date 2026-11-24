# Releases

- Versions are semver tags `vX.Y.Z`; `scripts/bump-version.sh` sets `Cargo.toml` and
  makes the tag. Releases are GitHub **pre-releases** while the version is 0.x.
- Pushing a tag runs the Release workflow (`.github/workflows/release.yml`, decided
  2026-11-20): each platform builds on its own GitHub machine with `scripts/release.sh`,
  and one job publishes the pre-release with every archive and a combined
  `SHA256SUMS.txt`. The tag must match `Cargo.toml`'s version or the build fails.
  - macOS: a universal `FarSub.app` (Apple Silicon and Intel), ad-hoc signed.
  - Windows: `farsub.exe` for x86_64.
  - Linux: `farsub` for x86_64 in a `.tar.gz`, built on Ubuntu 22.04 so it runs on
    distributions with that glibc or newer.
- The release notes start as GitHub's generated list of changes and are edited by hand.
- Running the workflow by hand only builds (the archives stay as the run's artifacts);
  `scripts/release.sh` still builds locally for any platform the machine can.
