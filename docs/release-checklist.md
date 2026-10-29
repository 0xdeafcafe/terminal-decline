# Release checklist: 0.1.0

The steps for the first public beta, in order (roadmap E.16). **[Alex]**
marks a step only Alex can do (the Apple secrets, the `release`
environment, making things public, tagging); the others can be done by an
agent in a branch. Nothing here has been run: the repository and the tap
are private, and no tag exists.

How the release works: pushing a tag `v*` runs
`.github/workflows/release.yml`: `build` (builds, signs with the
Developer ID certificate if the secrets exist, notarizes, zips) then
`sign` (the `release` environment: signs the update with the Sparkle key
and writes the appcast) then `release` (GitHub release with the zip, the
dSYMs, the pin stub, the appcast; release notes from git-cliff; commits the
cask to the tap). The same workflow without a tag builds the nightly.

## 1. Before anything is public

1. **[Alex] Apple secrets.** Repository secrets (Settings, Secrets and
   variables, Actions), read by the `build` job:
   `APPLE_CERT_P12` (base64 of the Developer ID Application certificate
   and key, as .p12), `APPLE_CERT_PASSWORD`, `APPLE_TEAM_ID`, `APPLE_ID`,
   `APPLE_APP_PASSWORD` (an app-specific password for notarytool).
   Without them the build is ad-hoc signed and the `sign` job refuses to
   publish an update.
2. **[Alex] The `release` environment.** Settings, Environments, create
   `release` (add a required reviewer if wanted, so a tag waits for a
   click). Put `SPARKLE_ED_PRIVATE_KEY` in it, and make `APPLE_TEAM_ID`
   available to it (repository secrets are). The key must be the private
   half of `packaging/sparkle-public-key`; the workflow checks this and
   fails if it isn't. Keep a backup of the key: installed copies trust
   only this key, forever.
3. **[Alex] Tap deploy key.** Create an SSH key pair; add the public half
   as a deploy key with write access to `0xdeafcafe/homebrew-tap`, and the
   private half as the repository secret `HOMEBREW_TAP_DEPLOY_KEY`.
   Without it the release skips the cask update.
4. Check the roadmap's N.2: all workflows green on `main`, Dependabot PRs
   merged or closed, and the nightly workflows (including the macOS 14
   smoke job) passing.
5. **[Alex] Dry run without a tag.** Run the Release workflow by hand
   (Actions, Release, Run workflow) on `main`. Check the build job's
   `signing` and `notarized` outputs say `developer-id` and `true`, and
   that the `spctl` and `stapler` checks pass. This publishes only a
   workflow artifact, not a release.
6. Smoke the dry run's zip on a Mac (ideally one running macOS 14, as
   well as the newest): unzip, open it, `spctl --assess --type exec -vv`
   says `Notarized Developer ID`, open a tab and a split, make a pin,
   `terminal-decline pin --list`.
7. Read the cask once more, `packaging/homebrew/terminal-decline.rb`:
   `caveats` (pins need the app installed; `brew upgrade --greedy`) and
   `zap` (config, caches, logs, preferences, saved state, and the
   `decline` leftovers). Check each zap path against what the app writes.

## 2. Prepare the release commit

8. Set the version to `0.1.0` in `Cargo.toml` (`[workspace.package]`,
   now `0.0.1`) and let `Cargo.lock` follow (`cargo check --locked` fails
   until it does). The tag sets the shipped version, but nightlies and
   `cargo install` take it from here.
9. Regenerate the changelog for the tag:
   `git cliff --tag v0.1.0 --output CHANGELOG.md`. Read it; fix the
   commit subjects that read badly by editing the changelog only if
   unavoidable (the release notes come from the commits, not this file).
10. Update the README's Status section (it says pre-release, and that the
    repository is private), and DESIGN.md for the end of the E phase, as
    the roadmap asks. Mark E.16 done in `docs/roadmap.md`.
11. `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D
    warnings`, `cargo test --workspace`; merge to `main`; wait for CI.

## 3. Make it public

12. **[Alex] Audit before the history is visible.** Search the history for
    secrets and private paths (`gitleaks detect`, or by hand), check
    `THIRD-PARTY-NOTICES.md` and `LICENSE`, and the cask's and README's
    URLs.
13. **[Alex] Make `0xdeafcafe/terminal-decline` public.** Then turn on
    what public repositories get: branch protection on `main`, secret
    scanning and push protection, Dependabot alerts. The tap is shared
    with another project: make it public only if that project is
    ready too (a Homebrew tap must be public for `brew tap` to work for
    anyone else).
14. **[Alex] Make `0xdeafcafe/homebrew-tap` public** (it holds
    `Casks/terminal-decline.rb` after the release, and a
    `cask_renames.json` entry from the old name `decline`).

## 4. Tag and release

15. **[Alex] Tag.** From the merged `main`:
    `git tag -a v0.1.0 -m "Terminal Decline 0.1.0"` and
    `git push origin v0.1.0`. This starts the release; it can't be taken
    back cleanly (an appcast item with a build number can't be reused).
16. **[Alex] Approve the `sign` job** if the `release` environment has a
    reviewer. Watch the run: `build`, `sign`, `release`. The first
    release has no appcast to compare with, so the `sign` job says "this
    is the first one" and publishes one if signing and notarization worked.
17. Check the GitHub release: the zip, `.sha256`, the dSYM zip, the pin
    stub zip, `appcast.xml`, and notes grouped by type and module.
    Edit the notes by hand if they need a sentence about what the beta is.
18. Check the tap: `Casks/terminal-decline.rb` has version `0.1.0` and
    the zip's SHA-256 from the release notes.

## 5. After the release

19. On a clean Mac (or a macOS 14 VM): `brew install --cask
    0xdeafcafe/tap/terminal-decline`, open it, `terminal-decline
    --version`, make a pin, then `brew uninstall --zap --cask
    terminal-decline` and confirm the paths in `zap` are gone.
20. Check the updater once a later build is published (a `0.1.1` or a
    test release): a running 0.1.0 installs it when quit. The app's
    update log (`update.log`, see docs/design/updater.md) shows the check.
21. Update the README's Status (no longer private, link the release) and
    note the release in `docs/roadmap.md`.
