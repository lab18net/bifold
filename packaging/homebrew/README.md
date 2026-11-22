# Publishing Bifold through Homebrew

Everything needed to ship Bifold as Homebrew formulae lives in this repository;
the tap itself is a second, tiny repository (`lab18net/homebrew-tap`). Why a tap
and not `homebrew/core`: see [ADR-006](../../docs/decisions/ADR-006-homebrew-distribution.md).

```
packaging/homebrew/
  bifold.rb.in        formula template: the terminal UI  (crates/tui, man/bifold.1)
  bifold-gui.rb.in    formula template: the desktop UI   (crates/gui, man/bifold-gui.1), macOS only
  cask/bifold-app.rb.in   cask template: the prebuilt Bifold.app from the GitHub release
  tap/                the files the tap repository starts with (README + Homebrew's workflows)
scripts/
  release.sh              bump version, date CHANGELOG, check, commit, tag (push optional)
  update-formula.sh       render the templates for a version into a tap checkout
  check-release-metadata.sh  versions, URLs, changelog, man pages, templates (CI runs it)
  changelog-section.sh    print one CHANGELOG section (release notes)
man/                  bifold.1, bifold-gui.1 (a test of the flags is in check-release-metadata.sh)
```

The templates hold the placeholders `@URL@` and `@SHA256@` (the cask also
`@VERSION@`); they are the only things that change per release.

The cask is named `bifold-app`, not `bifold`: a formula and a cask with the same
name in one tap make `brew install bifold` ambiguous. It downloads
`Bifold-<version>-aarch64.app.zip` from the GitHub release, so **the release must
be published before `update-formula.sh` can write the cask** (the script stops
with a message otherwise). Because `Bifold.app` is ad-hoc signed and not
notarised, the cask clears the quarantine flag in `postflight_steps`; drop that
step once releases are notarised. Casks are Apple silicon only for now, matching
the CI runner that builds the zip.

## One-time setup

1. Create the tap repository. It must be public and named `homebrew-tap`:

   ```bash
   gh repo create lab18net/homebrew-tap --public --description "Homebrew formulae for Lab18 software"
   git clone git@github.com:lab18net/homebrew-tap.git ../homebrew-tap
   cp -R packaging/homebrew/tap/. ../homebrew-tap/      # README.md and .github/
   ```

2. In the tap's GitHub settings: *Actions → General → Workflow permissions →
   Read and write*, and tick *Allow GitHub Actions to create and approve pull
   requests* (the bump workflow opens PRs, `pr-pull` pushes bottles).
3. Publish the first release of Bifold (next section), then create the first
   formulae:

   ```bash
   scripts/update-formula.sh 0.1.0 --tap ../homebrew-tap
   cd ../homebrew-tap && git add -A && git commit -m "bifold 0.1.0, bifold-gui 0.1.0" && git push
   ```

   Pushing straight to `main` is fine for the very first formula. Later changes
   go through pull requests so `brew test-bot` builds bottles for them.
4. Check it from a clean state: `brew install lab18net/tap/bifold && brew test bifold`.

## Releasing a new version

```bash
# write the release notes under [Unreleased] in CHANGELOG.md, commit, then:
scripts/release.sh 0.2.0 --dry-run     # preflight, shows what would change
scripts/release.sh 0.2.0 --push        # bump, date, make check, commit, tag, push
```

The tag starts `.github/workflows/release.yml`: it checks the metadata, runs the
tests, builds the release binaries and publishes the GitHub release (notes from
the changelog, plus a macOS tarball with its checksum). Its job summary prints the
SHA-256 of the source archive that Homebrew will download.

Then the tap takes over:

- **Automatic.** The tap's daily `brew bump` workflow (Homebrew's generated
  `autobump.yml`) should notice the new release and open a pull request that
  updates `url` and `sha256`; this has not been exercised yet, so confirm it on
  the first release and fall back to the manual way below if it stays quiet. Its
  `brew test-bot` workflow builds and tests the formulae (and bottles); run the
  `brew pr-pull` workflow with that PR's number to publish the bottles and merge.
- **By hand**, if you do not want to wait:
  `scripts/update-formula.sh 0.2.0 --tap ../homebrew-tap`, commit, open a PR.

## Trying a formula before anything is published

This is exactly what was done to validate the templates; it builds from source
on your machine and leaves nothing behind when you clean up.

```bash
export HOMEBREW_NO_AUTO_UPDATE=1
git ls-files -co --exclude-standard > /tmp/files.txt
tar -czf /tmp/bifold-0.1.0.tar.gz -s ',^,bifold-0.1.0/,' -T /tmp/files.txt   # bsdtar; GNU tar: --transform
brew tap-new --no-git lab18net/bifold-local
scripts/update-formula.sh 0.1.0 --tarball /tmp/bifold-0.1.0.tar.gz \
    --app-zip dist/Bifold-0.1.0-aarch64.app.zip \
    --tap "$(brew --repository)/Library/Taps/lab18net/homebrew-bifold-local"

brew style lab18net/bifold-local/bifold lab18net/bifold-local/bifold-gui
brew trust --tap lab18net/bifold-local            # Homebrew 7 will not load untrusted taps
brew audit --strict --online lab18net/bifold-local/bifold lab18net/bifold-local/bifold-gui
brew install --cask lab18net/bifold-local/bifold-app   # needs --app-zip dist/Bifold-0.1.0-aarch64.app.zip above
brew install lab18net/bifold-local/bifold         # compiles with Homebrew's Rust, ~1 min
brew test lab18net/bifold-local/bifold
man bifold                                         # the man page is installed

# clean up (also remove the trust entries from ~/.homebrew/trust.json)
brew uninstall bifold bifold-gui
brew untap lab18net/bifold-local
```

## Things to know

- **No bottles at first.** Until the tap's CI has built bottles, `brew install`
  compiles Bifold on the user's machine with Homebrew's Rust (`depends_on "rust"
  => :build`): about one minute for the terminal UI and just over a minute for
  the desktop one.
- **The GUI is a separate formula** so terminal-only users do not build `eframe`,
  and it is `depends_on :macos`: on Linux it would need X11/Wayland libraries that
  are not declared yet.
- **`std_cargo_args` implies `--locked`**, so `Cargo.lock` must always be
  committed and current; `check-release-metadata.sh` verifies it.
- **Renaming the product** (`scripts/rename-product.sh`) renames the man pages and
  templates too, but not the GitHub repository, the tap or old tags.
- **Moving to `homebrew/core` later**: it needs a notable project (see the ADR),
  a stable tagged release and a working `livecheck`; the formulae here already
  build from a checksummed tag archive with a locked dependency set.
