# Windows releases and updates

Every normal push to main runs CI and automatically publishes a stable Windows release after all checks pass. The repository is public, so installers and the update feed are available anonymously after publication. No update server or embedded GitHub token is needed.

## What CI does

1. Create a version commit from the pushed main revision and tag it `v<major>.<minor>.<workflow-run-number>`. All five version files agree. Main keeps its development baseline; the tag contains the exact built source. Run retries reuse the tag instead of rewriting it. Before run number 65536, change the version-stamping algorithm to introduce a new bounded patch sequence and advance the release series; changing the minor version alone does not reset GitHub’s run counter.
2. Rebuild and verify pinned FFmpeg, run frontend/Rust/packaging checks, build the installer, and sign its updater artifact using `TAURI_SIGNING_PRIVATE_KEY`. Branch/PR checks do not receive this secret and build unsigned artifacts.
3. Exercise installed smoke, reinstall/uninstall persistence, offline desktop workflows, and updater fixtures. Main releases also verify a signed installer handoff through the native updater, automatic restart, and retained settings, history, audio, and credentials in a disposable hosted profile.
4. Publish only after all jobs pass and only if the source revision is still main's head. Assets include the EXE, updater signature, latest.json, project source, exact FFmpeg source, MPL dependency sources, license/build recipe archive, and SHA256SUMS.txt. A failed check means no release. Existing published releases are never overwritten.

Update `docs/RELEASE_NOTES.md` when behavior changes; CI uses it for both GitHub and in-app notes. Ordinary releases need no manual version bump. Change the major/minor development baseline consistently when starting a new release series. `python scripts/release.py` checks versions. `python -m unittest discover -s tests/release -v` checks packaging/version stamping.

The workflow uses GitHub's built-in token. Prepare/publish jobs receive contents write; other jobs have read access. Keep Actions enabled and permit the workflow to create tags/releases. Do not add provider keys to CI. A manual `acceptance_run_id` dispatch diagnoses desktop tests using retained artifacts and never publishes.

## Routine maintainer workflow

1. Implement on a working branch, update `docs/RELEASE_NOTES.md` for behavior changes, and run the checks appropriate to the change.
2. Review and merge/push to main. **Pushing main is a deployment action**: it authorizes automatic publication after verification. Watch the Windows workflow in Actions; a successful push alone does not mean a downloadable build exists yet.
3. Confirm the new stable release has its installer, signature, update feed, source/license archives, and checksums. A superseded, canceled, or failed run may leave a tag without a release and consumes a patch number; gaps are normal.

Only a main **push** publishes. Pushing a tag or manually dispatching checks does not publish a new release. Diagnostic artifact reuse is for investigating tests, not releasing new application code. Investigate a failed publication before retrying; check whether a release and its complete assets already exist.

With the main-branch ruleset enabled, documentation-only pull requests must also pass required CI. Do not use `[skip ci]`: it leaves required checks pending and prevents merging. Merging to main still triggers the normal release workflow.

### Main-branch protection

Import [main.ruleset.json](../.github/rulesets/main.ruleset.json) through GitHub **Settings → Rules → Rulesets → New ruleset → Import a ruleset**, review it, and save. The file is configured as Active; committing it does not enable protection by itself.

The ruleset targets `main`, blocks deletion and force-pushes, requires a pull request with resolved review conversations, and requires an up-to-date branch with passing `prepare`, `media`, `windows`, and `desktop` checks from GitHub Actions. All four checks are required so an upstream failure cannot be hidden by a skipped downstream job. `release` is intentionally excluded because it runs only after a main push.

There are no bypass actors. Required approvals are zero so a solo maintainer can merge their own pull requests; another person's approval, code-owner review, and last-push approval are not required. Signed commits, linear history, and a merge queue are not required. Existing merge methods remain available. If another regular maintainer joins, consider requiring one approval.

This ruleset does not target release tags, so CI can continue creating version tags. The release workflow does not push commits to main. Keep required check names aligned with the workflow if jobs are renamed. An administrator can edit or disable the ruleset for recovery; normal contributions follow the same rules for everyone. See [GitHub's ruleset import instructions](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/managing-rulesets-for-a-repository#importing-a-ruleset).

### Development versions versus shipped versions

For example, main can say `1.5.0` while workflow run 18 ships `1.5.18`. CI creates a separate version-only commit and immutable `v1.5.18` tag; it does not commit that patch number back to main. Do not “repair” main to match the latest EXE. Use the release tag to reproduce a shipped build.

The five version files are `package.json`, `package-lock.json` (root and root-package entries), `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and the x-talk package entry in `src-tauri/Cargo.lock`. Change their major/minor baseline together when starting a release series. `scripts/prepare-release.py` is CI orchestration that **commits and pushes a tag**, not a harmless local version-check command.

### Where installers live

| Build | Location / purpose |
| --- | --- |
| Hosted main release | GitHub Releases is the authoritative distribution; CI builds independently of any maintainer’s WSL or Windows checkout. |
| Local Windows production build | `src-tauri/target/release/bundle/nsis/x-talk_<version>_x64-setup.exe` inside the Windows checkout; uses that checkout’s version. |
| Local debug/acceptance build | `src-tauri/target/debug/x-talk.exe`; acceptance uses a separate identifier and package version `0.0.0`. Never distribute it. |
| Local downloaded release | Keep downloads in an ignored directory such as `.tmp/releases/<tag>/` on WSL. Download the EXE and `SHA256SUMS.txt` from the same release and compare the EXE’s SHA-256 before handing it over. |

`C:\temp` is accessible from WSL as `/mnt/c/temp`. Copying a verified installer there is a manual convenience for local acceptance, not a deployment step. Future main pushes publish to GitHub; they do **not** copy files to this machine. Keep the release filename unchanged so the version is visible. Installing is a separate action; do not silently replace an owner’s running app while preparing artifacts.

The Meetings media bundle includes audio decoding for AAC, MP3, FLAC, Opus, Vorbis, ALAC and supported PCM, with MP4/MOV, Matroska/WebM and Ogg demuxing. Video is read only to extract its first audio stream; no video decoder or network protocol is enabled. Rebuild from the pinned recipe and update checksums/notices together when changing these capabilities.

## Signing and recovery

The updater's public key is committed in tauri.conf.json. Its private key belongs in the repository Actions secret `TAURI_SIGNING_PRIVATE_KEY`; the current key has an empty password. Keep a protected backup outside Git. Never log or commit it. Losing this key prevents existing installations from trusting newly signed updates; distribute a manual installer if recovery is necessary. Rotate keys deliberately, with an upgrade path for installed clients.

Updater signing is separate from Windows Authenticode. Windows still sees an unsigned publisher and may show SmartScreen warnings. Do not advise disabling security protections. macOS remains unsupported.

For local signed packaging, build with `src-tauri/tauri.release.conf.json` and the signing environment variables, then use `scripts/release.py --updater --tag v<version> --installer <exe> --ffmpeg-source <archive> --output <empty-directory>` from clean committed matching source. Normal `npm run desktop:build` remains available without private keys for contributors.

Fix a bad release with a new main commit and higher version, not a moved tag or replaced binary. The client installs only newer versions. A maintainer can remove a bad release from latest while preparing a replacement; do not silently roll clients back.

## Public distribution checks

Review release artifacts, README screenshots, notices, and history for private content. Verify anonymous download/update URLs and enable private vulnerability reporting in GitHub Code security settings. Hardware microphone/shortcut acceptance, provider live checks, and Windows 11 coverage remain distinct from hosted synthetic tests. Keep test evidence in `.tmp/`; record durable product decisions in SPEC.

The final visibility change is an explicit owner action. After it, check the Releases download link and `https://github.com/alexander-north/x-talk/releases/latest/download/latest.json` without GitHub authentication, follow the manifest’s immutable installer URL, and check for updates from an installed production app. Existing published releases become publicly accessible with the repository; another build is not required just for visibility. Never embed a PAT to work around private downloads.

Automatic publication is not automatic installation: clients check after 30 seconds and every 12 hours, unless disabled, and the user confirms Update and restart. Older builds without the updater need one manual installer upgrade. Check anonymous availability after publication; authenticated maintainer downloads alone do not establish public availability.

The public repository starts with a fresh history and the 1.6 release series. Its Actions run counter restarts, so advancing the minor version keeps every new release newer than the previous 1.5 installations. Reuse the existing signing key and updater URL to preserve update compatibility.
