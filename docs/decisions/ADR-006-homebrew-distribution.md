# ADR-006: Distribution through a Homebrew tap

## Context

Bifold is meant to be installed with one command on macOS (and Linux). Homebrew
is the expected channel for a command-line tool. Facts that shaped the decision,
checked on 2026-11-22:

- The name `bifold` is free in both `homebrew/core` and `homebrew/cask`.
- `homebrew/core` requires "public interest beyond its author": a GitHub project
  normally needs 30 forks, 30 watchers or 75 stars, and **90 / 90 / 225 for a
  self-submission by the repository owner**; a repository younger than 30 days
  is normally not eligible; a working `livecheck` and a stable tagged release are
  also needed. Homebrew itself points software that does not meet this at
  third-party taps. The Bifold repository was a few hours old with no stars.
- Homebrew 7 will not load formulae from an untrusted third-party tap
  (`brew trust`); installing by the full `user/tap/formula` name trusts it.
- Release builds are cheap: the terminal UI compiles in about a minute and
  installs to 1.5 MB; the desktop UI (egui/eframe) takes just over a minute and
  installs to 5.7 MB.
- The old manifest pointed `repository` at the wrong GitHub organisation and
  `homepage` at `lab18.io/bifold`, which does not resolve; `brew audit --online`
  rejects a dead homepage.

## Decision

1. **A personal tap, `lab18net/homebrew-tap`**, not `homebrew/core` (yet).
   Install line: `brew install lab18net/tap/bifold`. Revisit `homebrew/core`
   once the project meets the notability thresholds; the formulae already build
   from a checksummed tag archive with a locked dependency set, which core
   requires.
2. **Source formulae.** Each builds the tagged GitHub source archive with
   `cargo install` (`std_cargo_args`, which implies `--locked`) and installs the
   man page. Bottles are produced by the tap's own `brew test-bot` / `brew
   pr-pull` workflows (Homebrew's generated ones, pinned by commit), so users
   normally download a binary and the formula stays auditable source.
3. **Two formulae.** `bifold` (terminal UI; the default) and `bifold-gui`
   (desktop window; `depends_on :macos` because its Linux runtime libraries are
   not declared). Terminal-only users do not build `eframe`.
4. **The formulae live here as templates** (`packaging/homebrew/*.rb.in`, with
   `@URL@` and `@SHA256@`) and are rendered by `scripts/update-formula.sh`, so
   they are versioned and reviewed with the code and tested before any tap
   exists (see `packaging/homebrew/README.md`). The tap repository holds only the
   rendered `Formula/` files and Homebrew's workflows.
5. **Releases are tag-driven.** `scripts/release.sh` is the only thing that
   changes the version (workspace `Cargo.toml`), dates `CHANGELOG.md` and the man
   pages, runs `make check`, commits and tags. Pushing `vX.Y.Z` runs
   `.github/workflows/release.yml`, which publishes the GitHub release with
   notes from the changelog and a macOS binary tarball. The tap's daily `brew
   bump` workflow opens the formula PR, so this repository needs no token for the
   tap.
6. **`scripts/check-release-metadata.sh` guards drift** (run by CI on every push
   and by the release workflow): one workspace version, manifest URLs matching
   the git remote, a changelog section, man pages that lint and document every
   command-line flag, formula templates that still point at real crates, and an
   up-to-date `Cargo.lock`.
7. **Not published to crates.io** (`publish = false`): installation is by
   Homebrew, the GitHub release, or `cargo install --git`. This also avoids
   claiming generic crate names (`bifold-core`, …) on crates.io by accident.
8. **A cask, `bifold-app`, for the prebuilt `Bifold.app`.** Named differently from
   the formula `bifold` because a same-named formula and cask are ambiguous in
   one tap. It installs the zip attached to the GitHub release (Apple silicon
   only), and strips the quarantine attribute in `postflight_steps` because the
   app is only ad-hoc signed. Homebrew's official cask repository leans on
   signing and notarisation as its trust model (Homebrew's security docs), so a
   tap is the realistic home until a Developer ID exists. Local tests
   pass `brew style` and an install/uninstall (`--zap`) cycle with a `file://`
   URL; `brew audit` only complains about that unversioned local URL.
9. **Man pages** `bifold(1)` and `bifold-gui(1)` ship with the formulae.

## Consequences

- Users need one extra word (`lab18net/tap/`) and, on Homebrew 7, the tap's
  trust prompt; in return nothing here depends on Homebrew's review queue.
- Until bottles exist, the first install compiles Bifold with Homebrew's Rust
  (about a minute). Bottles for macOS and Linux appear once the tap's CI runs on
  the first formula PR.
- The tap needs a one-off setup and Actions permissions (documented), and the
  first release must be tagged before a formula can reference it: the templates
  deliberately carry placeholders rather than a made-up checksum.
- The product name appears in file names (`man/bifold*.1`,
  `packaging/homebrew/bifold*.rb.in`), formula class names and the repository
  slug. `scripts/rename-product.sh` updates the files; renaming the GitHub
  repository and the tap is manual.
- A prebuilt tarball is attached to each GitHub release for people who do not use
  Homebrew; it is built on the CI runner's architecture only (Apple silicon) and
  is not signed or notarised. `Bifold.app`
  (`scripts/package-macos.sh`) is built the same way and installed by the cask;
  notarisation is still open (docs/PLAN.md, M3).
