# Releases and updates

Every normal push to main runs CI and automatically publishes a stable release for Windows x64 and macOS on Apple Silicon after all checks pass. The repository is public, so downloads and the update feed are available anonymously after publication. No update server or embedded GitHub token is needed. One release always covers both platforms: if either platform fails, nothing is published.

## What CI does

The workflow is [.github/workflows/windows.yml](../.github/workflows/windows.yml) ("Build, verify and release"; the file name is kept because the ruleset and history refer to it).

1. `prepare` creates a version commit from the pushed main revision and tags it `v<major>.<minor>.<workflow-run-number>`. All version files agree. Main keeps its development baseline; the tag contains the exact built source. Run retries reuse the tag instead of rewriting it. Before run number 65536, change the version-stamping algorithm to introduce a new bounded patch sequence and advance the release series; changing the minor version alone does not reset GitHub’s run counter.
2. `media` rebuilds the pinned Windows FFmpeg tools on Ubuntu and verifies them. `windows` runs frontend/Rust/packaging checks and builds the installer without any signing key. `macos` (on `macos-15`, arm64) builds the macOS FFmpeg tools natively from the FFmpeg source archive the `media` job verified, runs the same checks where they apply (release unit tests, `npm run check`/`npm test`, `cargo fmt --check`, clippy with `-D warnings`, `cargo test --features desktop`), builds the `.app` and `.dmg` without any signing key, creates the updater archive, and smoke-checks the bundle without launching it (Info.plist identity, version, minimum macOS and privacy strings; `codesign --verify --deep --strict`; ad-hoc signature and audio-input entitlement; `lipo` arm64 only; Mach-O minimum macOS 14.2; bundled FFmpeg identical to the tools built in this job and able to encode and probe a file from inside the bundle; the archive unpacks to an identical, validly signed bundle; the disk image verifies, mounts and contains the same app). Ignored hardware tests run nowhere in CI.
3. For a release, a separate step in each platform job that does nothing else signs the updater artifact with `tauri signer sign --app-version <version>`: the NSIS installer on Windows, `x-talk_<version>_aarch64.app.tar.gz` on macOS. These two steps are the only ones that receive `TAURI_SIGNING_PRIVATE_KEY`, so npm scripts, `build.rs` and proc-macros never see it. A following step (`scripts/release.py check-signature`) checks that each signature names the committed public key and is bound to the release version. Branch/PR checks do not receive this secret and build unsigned artifacts.
4. `desktop` (Windows only; tauri-driver has no macOS support) exercises installed smoke, reinstall/uninstall persistence, offline desktop workflows, and updater fixtures. Main releases also verify a signed installer handoff through the native updater, automatic restart, and retained settings, history, audio, and credentials in a disposable hosted profile. The macOS update handoff has no automated test; see [macOS status](#macos-status-and-consequences).
5. `release` runs only after a main push and only if `prepare`, `windows`, `desktop` and `macos` all succeeded, and only if the source revision is still main's head. `scripts/release.py publish-set` checks both platform artifact sets against their checksums, expected names, signatures, the pinned FFmpeg source and (for macOS) the FFmpeg build manifest, then writes one `latest.json` with `windows-x86_64` and `darwin-aarch64` entries and a `SHA256SUMS.txt` covering every asset. Existing published releases are never overwritten.

Release assets:

| Asset | Purpose |
| --- | --- |
| `x-talk_<version>_x64-setup.exe` and `.sig` | Windows installer; also the Windows updater artifact |
| `x-talk_<version>_aarch64.dmg` | macOS download (ad-hoc signed app, not notarized) |
| `x-talk_<version>_aarch64.app.tar.gz` and `.sig` | macOS updater artifact (the `.app` bundle, gzipped tar) |
| `x-talk_<version>_aarch64_ffmpeg-build.json` | SHA-256 of the macOS `ffmpeg`/`ffprobe` in this release, with the source and recipe checksums and the Xcode/clang version that built them |
| `latest.json` | Update feed for both platforms, with immutable tag URLs |
| `x-talk_<version>_source.tar.gz` | Project source at the release tag |
| `ffmpeg-<version>.tar.xz` | Exact FFmpeg source for the bundled tools on both platforms |
| `x-talk_<version>_dependency-sources.tar.gz` | MPL dependency sources |
| `x-talk_<version>_licenses-and-build.tar.gz` | Licenses, notices and the FFmpeg build recipe |
| `SHA256SUMS.txt` | Checksums of every asset above |

Update `docs/RELEASE_NOTES.md` when behavior changes; CI uses it for both GitHub and in-app notes. Ordinary releases need no manual version bump. `python scripts/release.py` checks versions. `python -m unittest discover -s tests/release -v` checks packaging, the two-platform feed, version stamping and the media lock.

The workflow uses GitHub's built-in token. Prepare/publish jobs receive contents write; other jobs have read access. Keep Actions enabled and permit the workflow to create tags/releases. Do not add provider keys to CI. A manual `acceptance_run_id` dispatch diagnoses Windows desktop tests using retained artifacts, skips the build jobs (including macOS), and never publishes.

Workflow actions are pinned to full commit SHAs with a `# vX.Y.Z` comment; `.github/dependabot.yml` proposes weekly grouped updates for those pins and for npm and Cargo dependencies. Tauri packages are grouped separately; merge the npm `@tauri-apps/*` and Cargo `tauri*` updates together so the JS API, CLI and crates stay matched. `.github/workflows/audit.yml` runs `npm audit --omit=dev` and a pinned `cargo audit` weekly, on demand, and on pull requests that change a lockfile. It is not a required check and is outside the release workflow, so a new advisory cannot block a release; triage its failures as dependency updates.

macOS runner minutes cost several times Linux minutes. The `macos` job waits for the cheap `media` job, reuses its verified FFmpeg source archive, and builds everything once; there is no cargo cache (no new third-party actions). Expect it to be the longest job.

## Routine maintainer workflow

1. Implement on a working branch, update `docs/RELEASE_NOTES.md` for behavior changes, and run the checks appropriate to the change.
2. Review and merge/push to main. **Pushing main is a deployment action**: it authorizes automatic publication after verification. Watch the workflow in Actions; a successful push alone does not mean a downloadable build exists yet.
3. Confirm the new stable release has both platforms' downloads, the updater archives and signatures, the update feed with both platforms, source/license archives, and checksums. A superseded, canceled, or failed run may leave a tag without a release and consumes a patch number; gaps are normal.

Only a main **push** publishes. Pushing a tag or manually dispatching checks does not publish a new release. Diagnostic artifact reuse is for investigating tests, not releasing new application code. Investigate a failed publication before retrying; check whether a release and its complete assets already exist. Re-running only a failed `macos` job re-runs `release` after it; it never republishes an existing release.

With the main-branch ruleset enabled, documentation-only pull requests must also pass required CI. Do not use `[skip ci]`: it leaves required checks pending and prevents merging. Merging to main still triggers the normal release workflow.

### Main-branch protection

Import [main.ruleset.json](../.github/rulesets/main.ruleset.json) through GitHub **Settings → Rules → Rulesets → New ruleset → Import a ruleset**, review it, and save. The file is configured as Active; committing it does not enable protection by itself. **After this change, re-import it (or add `macos` to the existing ruleset's required checks by hand)**: GitHub does not read the file from the repository.

The ruleset targets `main`, blocks deletion and force-pushes, requires a pull request with resolved review conversations, and requires an up-to-date branch with passing `prepare`, `media`, `windows`, `desktop`, and `macos` checks from GitHub Actions. All are required so an upstream failure cannot be hidden by a skipped downstream job, and so a change that breaks the Mac build cannot reach main (where it would block every release). `release` is intentionally excluded because it runs only after a main push.

There are no bypass actors. Required approvals are zero so a solo maintainer can merge their own pull requests; another person's approval, code-owner review, and last-push approval are not required. Signed commits, linear history, and a merge queue are not required. Existing merge methods remain available. If another regular maintainer joins, consider requiring one approval.

This ruleset does not target release tags, so CI can continue creating version tags. The release workflow does not push commits to main. Keep required check names aligned with the workflow if jobs are renamed. An administrator can edit or disable the ruleset for recovery; normal contributions follow the same rules for everyone. See [GitHub's ruleset import instructions](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/managing-rulesets-for-a-repository#importing-a-ruleset).

### Development versions versus shipped versions

For example, main can say `1.5.0` while workflow run 18 ships `1.5.18`. CI creates a separate version-only commit and immutable `v1.5.18` tag; it does not commit that patch number back to main. Do not “repair” main to match the latest build. Use the release tag to reproduce a shipped build.

The version files are `package.json`, `package-lock.json` (root and root-package entries), `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and the x-talk package entry in `src-tauri/Cargo.lock`. The platform files `tauri.windows.conf.json` and `tauri.macos.conf.json` inherit the version and must not set their own (if one ever does, `prepare-release.py` stamps it and `release.py` requires it to agree); `src-tauri/Info.plist` must not set `CFBundleShortVersionString`/`CFBundleVersion`, and `bundle.macOS.bundleVersion` must stay unset or equal. Change the major/minor baseline together when starting a release series. `scripts/prepare-release.py` is CI orchestration that **commits and pushes a tag**, not a harmless local version-check command.

Scripts that read Tauri settings (`release.py`, `macos-bundle.sh`) use the merged view Tauri itself uses: `tauri.conf.json` overlaid with `tauri.<platform>.conf.json` as an RFC 7396 merge patch.

### Where builds live

| Build | Location / purpose |
| --- | --- |
| Hosted main release | GitHub Releases is the authoritative distribution; CI builds independently of any maintainer’s checkout. |
| Local Windows production build | `src-tauri/target/release/bundle/nsis/x-talk_<version>_x64-setup.exe` inside the Windows checkout; uses that checkout’s version. |
| Local macOS production build | `src-tauri/target/release/bundle/macos/x-talk.app` and `bundle/dmg/x-talk_<version>_aarch64.dmg`; `scripts/macos-bundle.sh` adds `bundle/macos/x-talk_<version>_aarch64.app.tar.gz`. |
| Local debug/acceptance build | `src-tauri/target/debug/x-talk(.exe)`; acceptance uses a separate identifier and package version `0.0.0`. Never distribute it. |
| Local downloaded release | Keep downloads in an ignored directory such as `.tmp/releases/<tag>/`. Download the file and `SHA256SUMS.txt` from the same release and compare its SHA-256 before handing it over. |

`C:\temp` is accessible from WSL as `/mnt/c/temp`. Copying a verified installer there is a manual convenience for local acceptance, not a deployment step. Future main pushes publish to GitHub; they do **not** copy files to this machine. Keep the release filename unchanged so the version is visible. Installing is a separate action; do not silently replace an owner’s running app while preparing artifacts.

## Bundled FFmpeg

The Meetings media bundle includes audio decoding for AAC, MP3, FLAC, Opus, Vorbis, ALAC and supported PCM, with MP4/MOV, Matroska/WebM and Ogg demuxing. Video is read only to extract its first audio stream; no video decoder or network protocol is enabled. Both platforms use exactly the same component list from [build-media.sh](../scripts/build-media.sh), built from the same official archive pinned by SHA-256 in [media.lock.json](../scripts/media.lock.json): an LGPL 2.1-or-later configuration with no GPL, nonfree or external libraries. Rebuild from the pinned recipe and update checksums/notices together when changing these capabilities.

- **Windows** (`ffmpeg.exe`, `ffprobe.exe`): cross-built with MinGW-w64 on Linux/WSL (`scripts/build-media.sh`), statically linked. The recipe and its pinned checksums are unchanged by macOS support.
- **macOS** (`ffmpeg`, `ffprobe`): built natively on Apple Silicon by the same script (`uname` selects the platform), arm64 only, deployment target macOS 14.2, with the FFmpeg libraries linked statically and only macOS system libraries linked dynamically; the script fails on any other dependency (`otool -L`, so never Homebrew), on a different architecture or minimum OS, and stubs pkg-config so no external library can be found. VideoToolbox, AudioToolbox and every other autodetected Apple framework stay disabled: the app needs only FFmpeg's own audio decoders, which keeps decoding identical on both platforms and avoids media-framework dependencies (the license posture would be the same either way, since system frameworks are not distributed). The stripped tools are ad-hoc signed (`codesign -s -`), which arm64 macOS requires.

The two platforms are verified differently:

- **Windows tools are pinned.** The MinGW cross-build is reproducible, so its checksums are recorded in the lock (`platforms.windows-x86_64.binaries`) and listed in `src-tauri/binaries/NOTICE.txt`. Every build must reproduce them; `scripts/build-media.sh --record` re-pins after a deliberate change.
- **macOS tools are verified per build, not pinned.** Apple's toolchain embeds its own version and the SDK in the binaries, and hosted runner images change Xcode versions without notice, so a committed checksum would make CI fail (and block every release) for reasons unrelated to the code. Instead, `scripts/build-media.sh` on a Mac always builds from the checksum-verified source with the committed recipe, checks the result (arm64 only, minimum macOS 14.2, only macOS system libraries, valid ad-hoc signature, runs), and writes `src-tauri/binaries/media-build.json` (git-ignored): the tools' SHA-256, the source and recipe checksums, and the toolchain. `verify-media.py`/`verify-media.mjs` (and so `desktop:build`/`desktop:debug`) accept only tools matching that manifest, built by the current recipe from the pinned source. In CI the same job builds the tools, bundles them, compares the bundled copies byte for byte with the build output, and `release.py` checks the updater archive against the manifest and publishes the manifest as `x-talk_<version>_aarch64_ffmpeg-build.json`. Any Xcode or Command Line Tools version works; no re-pinning is ever needed.

`verify-media.py` (build/CI) and `verify-media.mjs` (`desktop:build`) check the tools for the host platform (Linux/WSL checks the Windows tools; set `XTALK_MEDIA_PLATFORM` to override). The corresponding source for both platforms is the published `ffmpeg-<version>.tar.xz` plus the recipe in the licenses-and-build archive.

The trade-off: a macOS release proves which source and recipe produced its tools and that the shipped bytes are the ones built in that run, but not that an independent rebuild reproduces them byte for byte. That is the same assurance the app executable itself has. If you want more, add GitHub artifact attestations (`actions/attest-build-provenance`) for the `.dmg` and updater archive later.

## Signing and recovery

The updater's public key is committed in `tauri.conf.json` and applies to both platforms. Its private key belongs in the repository Actions secret `TAURI_SIGNING_PRIVATE_KEY`; the current key has an empty password. Keep a protected backup outside Git. Never log or commit it. Losing this key prevents existing installations from trusting newly signed updates; distribute a manual download if recovery is necessary. Rotate keys deliberately, with an upgrade path for installed clients.

Updater signing is separate from operating-system code signing. Windows still sees an unsigned publisher and may show SmartScreen warnings. Do not advise disabling security protections.

CI does not use `src-tauri/tauri.release.conf.json` (`createUpdaterArtifacts` makes `tauri build` require the private key, and on macOS would also produce a differently named archive). For local signed packaging, build normally and sign the updater artifact afterwards with `npm run tauri -- signer sign --app-version <version> <file>` (without `--app-version` clients reject the signature because `requireSignedVersion` is on). Then, from clean committed matching source:

- Windows: `python scripts/release.py windows --updater --tag v<version> --installer <exe> --ffmpeg-source <archive> --output <empty-directory>`
- macOS: `scripts/macos-bundle.sh`, sign the `.app.tar.gz`, then `python3 scripts/release.py macos --updater --tag v<version> --dmg <dmg> --app-archive <app.tar.gz> --media-manifest src-tauri/binaries/media-build.json --output <empty-directory>`
- Combined release: `python3 scripts/release.py publish-set --tag v<version> --windows <dir> --macos <dir> --output <empty-directory>` writes `latest.json` and `SHA256SUMS.txt`.

Normal `npm run desktop:build` remains available without private keys for contributors.

### macOS status and consequences

The macOS app is **not signed with an Apple Developer ID and not notarized**. The bundle is ad-hoc signed (`bundle.macOS.signingIdentity: "-"`) with the hardened runtime and the `com.apple.security.device.audio-input` entitlement. Consequences:

- Gatekeeper blocks the first launch of a downloaded copy. Users approve it once in **System Settings → Privacy & Security → Open Anyway** (README has the steps). Never advise disabling Gatekeeper or removing quarantine attributes with Terminal commands.
- An ad-hoc signature has no stable identity: every release has a new code hash. macOS may therefore ask again for Microphone and system-audio access, and for Keychain access to saved API keys, after each update. Organization-managed Macs may forbid running such apps at all.
- In-app updates download the archive directly (no quarantine attribute), verify it with the updater key, and replace the `.app` in place. This path has no automated test and must be checked by hand on a Mac (install an older release from its `.dmg`, update, confirm relaunch, settings, history and keys).
- The FFmpeg tools inside `Contents/Resources/binaries` are separately ad-hoc signed executables sealed into the bundle signature. Whether Gatekeeper evaluates them separately when the approved app starts them must be confirmed by importing a file in a freshly downloaded, approved copy.

### Adding Developer ID signing and notarization later

This needs an Apple Developer Program membership and a **Developer ID Application** certificate. Tauri's bundler signs and notarizes when these environment variables are present during `tauri build`:

- Signing: `APPLE_CERTIFICATE` (base64 of the exported `.p12`), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` (`Developer ID Application: <Name> (<TEAMID>)`), and in CI a `KEYCHAIN_PASSWORD` for the temporary keychain. Set `bundle.macOS.signingIdentity` in `tauri.macos.conf.json` to that identity (or remove `"-"` and rely on the variable).
- Notarization: either an App Store Connect API key (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH` to the `.p8`) or `APPLE_ID`, `APPLE_PASSWORD` (app-specific password) and `APPLE_TEAM_ID`.
- Keep `hardenedRuntime: true` and the entitlements file. Notarization rejects unsigned or ad-hoc Mach-O files anywhere in the bundle, so `ffmpeg`/`ffprobe` must be signed with the Developer ID, `--options runtime` and `--timestamp` before bundling (or moved to Tauri's `externalBin`, which the bundler signs). Their bytes then change: write the build manifest after signing (or teach `release.py` and `macos-bundle.sh` to compare the code without its signature).
- Store the certificate and notarization credentials as secrets of a GitHub Environment restricted to `main`, and expose them only to the signing/notarizing steps. Because Tauri signs during `tauri build`, keeping the build step secret-free means building unsigned, then signing, notarizing (`xcrun notarytool submit --wait`) and stapling (`xcrun stapler staple`) the `.app` in a separate step, and producing the `.dmg` afterwards (for example with `tauri bundle --bundles dmg`) and notarizing it too. Create the updater archive from the stapled app.
- Then remove the Open Anyway instructions from README and update the consequences above. A Developer ID identity also gives the app a stable code identity, so permissions and Keychain access survive updates (after one final re-approval).

### Local macOS build

On an Apple Silicon Mac with macOS 14.2+, Xcode or the Command Line Tools (any recent version), Rust (rustup installs the pinned 1.93.1), Node.js 24.11.1 and Python 3.11+:

```sh
npm ci
scripts/build-media.sh                 # native ffmpeg/ffprobe and src-tauri/binaries/media-build.json
python3 scripts/generate-notices.py
npm run desktop:build                  # .app and .dmg, ad-hoc signed
scripts/macos-bundle.sh                # updater archive and smoke checks
```

For day-to-day development, Homebrew's FFmpeg is enough (see README); `desktop:build` deliberately refuses tools that `scripts/build-media.sh` did not build.

### Recommended repository settings

These cannot be set from the repository and are owner actions:

- Move `TAURI_SIGNING_PRIVATE_KEY` from a repository secret into a GitHub Environment (for example `release`) whose deployment branches are limited to `main`. Environment secrets are only available to jobs that declare that environment, so this first needs the two signing steps moved into their own job that declares it and hands the `.sig` files back as artifacts; that restructuring is a planned follow-up, not done yet. Moving the secret before then makes main releases fail closed at the signing steps.
- Give the signing key a password. Re-encrypt the existing key rather than generating a new one (the Tauri CLI only generates new keys; a minisign-compatible tool can change a key's password), confirm locally that a signature made with it still verifies against the committed public key, store the password as a second secret, and pass it as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in both signing steps instead of `''`.
- Enable Dependabot alerts and security updates in **Settings → Code security**.

Fix a bad release with a new main commit and higher version, not a moved tag or replaced binary. The client installs only newer versions. A maintainer can remove a bad release from latest while preparing a replacement; do not silently roll clients back.

## Public distribution checks

Review release artifacts, README screenshots, notices, and history for private content. Verify anonymous download/update URLs and enable private vulnerability reporting in GitHub Code security settings. Hardware microphone/shortcut acceptance, provider live checks, Windows 11 coverage and all hands-on macOS checks remain distinct from hosted synthetic tests. Keep test evidence in `.tmp/`; record durable product decisions in SPEC.

The final visibility change is an explicit owner action. After it, check the Releases download links and `https://github.com/alexander-north/x-talk/releases/latest/download/latest.json` without GitHub authentication, follow each platform's immutable URL in the manifest, and check for updates from an installed production app on each platform. Existing published releases become publicly accessible with the repository; another build is not required just for visibility. Never embed a PAT to work around private downloads.

Automatic publication is not automatic installation: clients check after 30 seconds and every 12 hours, unless disabled, and the user confirms Update and restart. Older builds without the updater need one manual installer upgrade. Check anonymous availability after publication; authenticated maintainer downloads alone do not establish public availability.

The public repository starts with a fresh history and the 1.6 release series. Its Actions run counter restarts, so advancing the minor version keeps every new release newer than the previous 1.5 installations. Reuse the existing signing key and updater URL to preserve update compatibility. The first release with macOS support is the first macOS version; there is no earlier Mac build to upgrade from.
