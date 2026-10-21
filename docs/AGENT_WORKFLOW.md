# Working on x-talk as an agent

This is a maintainer guide for an agent starting without the original conversation. It records repeatable practices, not a snapshot of somebody's workstation or a promise that earlier tests cover your changes.

## Orient before editing

1. Inspect the branch, working tree, and relevant diff. Other work may already be in progress; do not reset it, stage it incidentally, or treat an uncommitted feature as a released contract.
2. Read [AGENTS.md](../AGENTS.md) and the relevant behavior in [SPEC.md](../SPEC.md). Clarify product decisions that affect scope; resolve routine implementation details yourself once direction is established.
3. Check [package.json](../package.json), [Cargo.toml](../src-tauri/Cargo.toml), [rust-toolchain.toml](../rust-toolchain.toml), and the [Windows workflow](../.github/workflows/windows.yml) for actual commands, features, and pinned versions. Do not upgrade tooling just to get started.
4. Use [README setup](../README.md#build-and-test) for prerequisites and initial builds. Use [RELEASING.md](RELEASING.md) for release work. Keep product decisions in SPEC, operational lessons here, and temporary evidence in `.tmp/`.

Windows x64 is the supported desktop target. WSL/Linux is useful for core Rust tests and building the bundled Windows media tools. A Linux test pass or a browser-only UI preview does not verify the Windows application.

## Code map

| Area | Start with | Responsibility |
| --- | --- | --- |
| UI and navigation | `src/App.svelte`, `src/components/`, `src/style.css`, `src/glass.css`, `src/orb-art.ts` | Recorder, recordings library, settings, shared controls and themes |
| Frontend state and IPC | `src/ipc.ts`, `src/types.ts`, `src/refresh.ts`, `src/settings.ts` | Typed native boundary, serialized/coalesced refreshes, revision-safe settings writes |
| Native startup and commands | `src-tauri/src/app.rs`, `commands.rs` | Desktop lifecycle, shortcuts, tray, command registration, state/progress responses |
| Domain and persistence | `domain.rs`, `storage.rs`, `src-tauri/migrations/` | Settings, jobs, immutable request snapshots, SQLite, history and retention |
| Capture and media | `capture.rs`, `spectrum.rs`, `media.rs` | Microphone/writer lifecycle, admission/recovery journals, validation and FFmpeg subprocesses |
| Meetings | `src-tauri/src/meetings/`, `src/meetings/`, `src/components/meetings/` | Separate SQLite library, durable jobs, WASAPI microphone/loopback capture, JSON projection and review UI |
| Processing | `jobs.rs`, `providers.rs` | Queue/retry/resume behavior, stage persistence, provider requests |
| Credentials and diagnostics | `credentials.rs`, `diagnostics.rs` | Windows credential storage and bounded local logs |
| Audio feedback | `cues.rs`, `sounds.rs`, `scripts/generate-sounds.py` | Cue scheduling, playback, reproducible original sound assets |
| Acceptance backend | `test_support.rs`, `tests/e2e/` | Deterministic debug-only providers and native UI tests |
| Updates | `src-tauri/src/updates.rs`, `src/components/Updates.svelte`, `src/updates.ts` | Signed update checks, installation guards, confirmation and status |
| Packaging | `scripts/prepare-release.py`, `scripts/release.py`, `generate-notices.py`, `build-media.sh` | Version/material validation, dependency sources/notices, pinned media executables |

Bare Rust filenames in the table are relative to `src-tauri/src/`. Provider and model support is defined in the domain/catalog and provider implementations, not in this guide.

## Preserve these boundaries

- Rust owns capture, jobs, persistence, credentials, and native effects. Update the frontend types/IPC and Rust command contract together when changing a boundary. For new IPC commands, also update `src-tauri/permissions/application.toml`; registering a handler alone does not authorize the main window to invoke it. `src/ipc-permissions.test.ts` checks frontend commands against the main-window allowlist.
- The `capture-spectrum` event carries bounded frequency bands separately from state refresh. Admission remains native and serialized across all processing entry points. Cancellation must acknowledge the dropped worker future before releasing its operation; late writes must not revive canceled jobs.
- Recording progress uses the lightweight `progress` response. Do not make each progress event enumerate microphones, read credentials, reload prompts, or load a history page. Full metadata refreshes still belong at startup and configuration changes; preserve refresh serialization/coalescing.
- Keep revision checks and immutable job snapshots. A settings edit must not silently rewrite the configuration of already queued work.
- Real-time recordings stream through `realtime.rs` while the capture writer preserves the original. A bounded queue must fail explicitly on overflow, never drop audio or block the microphone callback. Only committed text may become a durable result. Stop waits for the final commit; Cancel closes the stream without delivery. Imports and explicit retries remain batch jobs. Recovery may reuse a durable successful streaming outcome, but must never silently reopen a paid session.
- Resume and explicit retry have different billing implications. An uncertain provider response must not become an automatic new paid request. Review stage-specific recovery tests when changing jobs/providers.
- Preserve original audio, validate media, and retain subprocess supervision. Performance improvements should remove redundant work rather than bypass validation or change retention semantics.
- Capture cancellation, normal Stop, malformed-journal recovery, and Quit are separate paths. Every one has previously exposed bugs that other paths did not catch.
- Diagnostics must not acquire transcript text, audio, credentials, or private filenames as part of debugging. Use synthetic fixtures for screenshots and logs intended for sharing.

## Choose verification by impact

Run focused checks while iterating, then the relevant broader checks before handoff. Do not rebuild installers after every CSS change or repeatedly rerun an unchanged, already-passing suite. Do broaden verification when a new failure or change crosses another boundary.

| Change | Useful verification |
| --- | --- |
| Documentation only | Check referenced files/links, examples against actual scripts, and `git diff --check`; no application rebuild required |
| Frontend state or forms | `npm test`, `npm run check`, `npm run build`; isolated native acceptance for IPC/native behavior |
| Rust/domain/storage/jobs | Core Rust tests plus relevant `workflow.rs` regressions; Windows desktop checks if native code is involved |
| Audio preparation | `src-tauri/tests/media.rs` with real FFmpeg; cover compression/silence combinations and original preservation |
| Capture, shortcuts, Quit, clipboard | Windows desktop checks and relevant native suites; real microphone/focus tests where required |
| Provider adapters | Request-level/unit coverage first; separately authorized live tests for network acceptance |
| Installer/resources/release scripts | Packaging regressions, native release build/guard, clean hosted installer lifecycle and offline acceptance |

The standard frontend and native commands are in [README](../README.md#build-and-test). In particular:

```sh
# WSL/Linux core tests; media tests need system FFmpeg/ffprobe.
cargo test --manifest-path src-tauri/Cargo.toml --locked

# Windows native tests, after preparing bundled media tools.
cargo test --manifest-path src-tauri/Cargo.toml --features desktop --locked

# Release-script regressions, without producing an installer.
python -m unittest discover -s tests/release -v
```

Media integration tests use bundled executables on Windows and `/usr/bin/ffmpeg` and `/usr/bin/ffprobe` on Linux by default; `XTALK_FFMPEG` and `XTALK_FFPROBE` override those paths. Do not confuse deterministic acceptance-provider media preparation with these real-media tests.

Offline meeting playback checks exercise real output when an endpoint exists, and assert the explicit no-output error path on hosted machines without sound hardware. Save-dialog acceptance verifies the actual path returned by the native download command; setting filenames through UI Automation can leave the suggested name unchanged on Windows Server. Meeting acceptance is separate from dictation. `tests/e2e/meetings.mjs` runs in the offline suite with real native media/storage/clipboard and synthetic providers. `XTALK_E2E_MEETING_RECOVERY=1` selects an isolated recovery suite with generated raw audio and no microphone or paid requests; it checks compressed admission, raw-file cleanup, and explicit processing of a recovered meeting. `cargo test --manifest-path src-tauri/Cargo.toml --test meetings --test meeting_media --locked` covers meeting domain, jobs and media. The ignored `meeting_media` tests prepare a four-hour synthetic WAV and archive two hours of synthetic capture tracks. On Windows, the ignored `meeting_capture` test uses real microphone and loopback devices; coordinate first. Set `XTALK_MEETING_CAPTURE_SECONDS=7200` for the duration check; it mutes both retained sources after the opening synthetic sample. Do not run native capture tests concurrently. `live_meetings` is an explicitly authorized, billable synthetic provider check; inspect its environment requirements before running it.

Meeting import admission precedes media preparation: the durable `importing` row is visible immediately, preparation runs behind the native import gate, and completion publishes audio/hash atomically. Startup marks interrupted imports failed without provider work. Keep navigation-during-import and failed-import card coverage in the native meeting suite.

Meeting library and periodic job queries use derived metadata/expression indexes so indefinite text history does not reload all word evidence. Full records belong in detail/export and active work. Meeting data lives in its own `meetings` subdirectory and must never be cleared by dictation fixtures. Unknown paid submissions require explicit retry; known AssemblyAI IDs resume polling. Capture journals and admission journals preserve local recovery. JSON exports omit managed paths and provider upload URLs.

Record the commands, operating system, build identity, and outcome. Test counts evolve: report the current results, not historical counts copied from SPEC. A Windows Server hosted test is not evidence for every Windows 10/11 device configuration.

## Native UI iteration

Use `npm run desktop:ui` for full-fidelity UI refinement in a native development window. `scripts/dev-ui.ps1` selects the optional project-local Node runtime, verifies media checksums, enables debug test providers and sets persistent data under `.tmp/ui-dev/data`. `src-tauri/tauri.ui-dev.conf.json` gives it a separate app identity and visible development title. The installed app can remain running. Frontend changes reload through Vite; Rust changes rebuild and restart. This is an interactive workspace, not a disposable acceptance-test profile. Provider output/credentials are synthetic, but capture, clipboard and playback still have real desktop effects. Use the isolated acceptance suites below for repeatable verification.

## Native test modes and isolation

The authoritative mode selection is [wdio.conf.mjs](../tests/e2e/wdio.conf.mjs). Inspect it before choosing `--spec` or changing test variables.

| Mode | Selection | What it exercises / constraints |
| --- | --- | --- |
| Offline acceptance | `XTALK_E2E_OFFLINE=1` | Generated WAV import, native UI/storage and deterministic provider results; no microphone or paid calls |
| Full capture acceptance | Unset OFFLINE and UPDATES; `XTALK_E2E_STOP=1`, `XTALK_E2E_CAPTURE=1` | Isolated providers but actual capture/writer and desktop behavior; needs a usable microphone and unlocked desktop |
| Cross-app hotkeys | `XTALK_E2E_SHORTCUTS=1` with the appropriate capture suite | Another application takes focus; coordinates with the user's keyboard/desktop use |
| Installed smoke | No OFFLINE/STOP/UPDATES flags; explicit `XTALK_APP_PATH` | Production executable, real settings/credentials environment; not automatically disposable |
| Updater acceptance | `XTALK_E2E_UPDATES=1` | Isolated feed/signature fixtures; actual installer handoff only in disposable hosted CI |
| Installer lifecycle | `upgrade.mjs`, `seed_upgrade.py`, workflow | Disposable hosted CI profile only; never spoof `GITHUB_ACTIONS` locally to bypass its guard |
| Live providers | Ignored tests in `src-tauri/tests/live.rs`; optional `XTALK_E2E_LIVE` smoke path | Billable requests and external data transfer; require explicit authorization and suitable fixtures |

`tests/e2e/realtime.mjs` joins the full capture suite and checks the native live preview, final text after normal Stop, and Cancel with isolated providers. The ignored `realtime_elevenlabs` test accepts an explicitly permitted 16-bit PCM WAV via `XTALK_LIVE_AUDIO`, streams it at microphone pace, and reports Stop-to-final latency. Keep its fixture at most 60 seconds; output goes to `.tmp/`. It uses the existing opt-in live-test credential loader. Do not run it without authorization for a billable ElevenLabs request.

For an authorized Windows comparison against the installed ElevenLabs credential, use the ignored `live_comparison` integration test. Set `XTALK_COMPARE_MANIFEST` to a private JSON file under `.tmp/` containing `clips` (up to four objects with `name`, absolute `pcm` and `original` paths, and `duration` in seconds), plus `compression` and `silence_removal` booleans. Each clip must be at most 60 seconds; `pcm` must be mono 16-bit PCM WAV at a supported real-time sample rate. The three arms use microphone-paced streaming, batch on the identical PCM file, and batch with the selected preparation settings. Each arm is a separate billable request. It writes private transcript/timing JSON receipts beside the manifest; do not upload or commit those receipts or fixtures.

Run `cargo test --manifest-path src-tauri/Cargo.toml --test live_comparison --locked -- --ignored --nocapture`. `XTALK_COMPARE_CLIP` selects one clip. `XTALK_COMPARE_ROUNDS=2` runs two rounds in reversed arm order; `XTALK_COMPARE_FIRST_ROUND=2` runs only the second round. Default is one round. Repeating a command makes new paid requests and replaces the corresponding receipts. Stop-to-final measurements exclude downstream transformation, clipboard delivery, and UI refresh. Accuracy requires an independently checked reference; agreement with another model output alone does not establish correctness.

For isolated UI work, build with `npm run desktop:test-build`. This applies the acceptance app identifier and enables `test-providers`; it is not the production installer. The runner supplies `XTALK_TEST_PROVIDERS=1` and a fresh `XTALK_TEST_DATA_DIR`, then checks native state to verify isolation. Do not remove that check. Release compilation with `test-providers` must fail.

After driver setup and the acceptance build, this PowerShell example makes the mode explicit and cleans up its environment afterward. Run it in a dedicated test shell:

```powershell
Remove-Item Env:XTALK_E2E_UPDATES,Env:XTALK_UPDATE_INSTALL,Env:XTALK_TEST_UPDATE_URL,Env:XTALK_APP_PATH,Env:XTALK_E2E_STOP,Env:XTALK_E2E_CAPTURE,Env:XTALK_E2E_SHORTCUTS,Env:XTALK_E2E_LIVE -ErrorAction SilentlyContinue
$env:XTALK_E2E_OFFLINE = '1'
try {
  npm run test:e2e
  if ($LASTEXITCODE -ne 0) { throw 'Offline acceptance failed' }
} finally {
  Remove-Item Env:XTALK_E2E_OFFLINE -ErrorAction SilentlyContinue
}
```

Use `XTALK_APP_PATH` only when deliberately testing a particular executable; otherwise the runner selects `src-tauri/target/debug/x-talk.exe`. Check the native `state` diagnostics (PID, version/build identity, test-provider status) when multiple copies or builds exist. The acceptance package uses version `0.0.0` for updater comparisons; diagnostics report the Cargo source version/build identity. Neither version alone proves that a process is the intended production or acceptance executable.

Coordinate actual microphone, clipboard, global shortcut, and focus tests with the user if they are using the same Windows desktop. An unlocked desktop and correct foreground application matter. Do not treat interference from another microphone user or a locked session as an application regression without checking.

The installed application normally uses `%LOCALAPPDATA%\com.xander.xtalk`, Windows Credential Manager, and `%TEMP%\x-talk\logs`. Never point destructive fixtures or Clear All tests at the owner's profile. General native test variables do not make a production-mode smoke run safe by themselves.

## WSL and Windows build workflow

Use WSL/Linux for `scripts/build-media.sh`, core Rust checks, and ordinary Git work. Use native Windows tools for desktop compilation, WebView2, microphone, clipboard, and installer testing. Follow the pinned prerequisites in README rather than relying on preinstalled defaults.

Keep the Windows build checkout on a Windows drive, for example `C:\dev\x-talk`. If working from a WSL checkout and mirroring into a Windows scratch workspace:

- Designate one source checkout as authoritative. Copy source/configuration, assets, prompts, license files and required media resources; do not copy `.env` or unrelated scratch data.
- Preserve each environment's own `node_modules` and Rust `target` directories for incremental builds. Do not share platform-specific dependency/build directories between Linux and Windows.
- Run Rust desktop integration tests before building the UI acceptance executable: Cargo can replace `target/debug/x-talk.exe` with a binary using different features/configuration. Re-run `desktop:test-build` before WebDriver if a later Cargo test built that binary.
- Do not synchronize files while the native compiler, packager, or tests are using them. After syncing, rebuild the affected executable before claiming to test the new source. Set `XTALK_GIT_COMMIT` to the source checkout’s eight-character Git revision (append `-dirty` for uncommitted builds) when building a mirror without Git metadata; do not substitute the source fingerprint. Git checkouts derive this automatically.
- Do not assume a scratch workspace is a Git checkout or suitable for `scripts/release.py`. Final release packaging requires a clean, committed source checkout matching the build.
- Keep local bootstrap/sync scripts in `.tmp/` and do not depend on a prior session's absolute paths, installed versions, or temporary helper scripts.
- Check native `node --version` against the workflow pin (currently 24.11.1). An older Windows Node earlier on PATH can break WebDriver dependencies even when the WSL Node version is correct.
- Prefer a PowerShell script file over deeply nested Bash/PowerShell quoting. Check `$LASTEXITCODE` after external commands; `$ErrorActionPreference` alone is not a reliable native-command exit check across PowerShell versions. Do not interpret a stale or unset `$LASTEXITCODE` after a pure PowerShell script as that script’s result.
- Track the PID of processes started for tests. Do not globally kill all x-talk, WebView2, Node, or PowerShell processes to clear a failed run.

Local installer paths, release downloads, and the WSL `/mnt/c/temp` ↔ Windows `C:\temp` handoff are documented in [RELEASING.md](RELEASING.md#where-installers-live). Hosted CI publishes downloads; local copies are not automatic.

Use native Git/SSH if it already provides the authorized repository identity. Git transport and the GitHub API can authenticate as different accounts. If a project-specific API token is supplied, pass it only to the relevant child process (for example via `GH_TOKEN`) without replacing global `gh` authentication. Do not print, source as shell code, or commit `.env`. The application does not load `.env`; the opt-in live Rust tests do.

## Known pitfalls and their fixes

| Symptom | Check / established fix |
| --- | --- |
| WebDriver reports browser/driver version mismatch | Match **WebView2**, not merely the installed Edge browser. Run `scripts/install-webdriver.ps1`; it selects the runtime and sets `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`. Run tests in that shell so the setting is inherited. The runner uses an explicit `--native-driver` path to avoid another driver earlier on PATH. |
| WebDriver treats a valid native result as an error | Wrap returned domain objects in `{ result: value }` inside `browser.execute` and unwrap afterward when the object has a top-level `error` field; preserve rejected IPC promises. |
| Native `invoke` is undefined at test startup | Wait for the native bridge as the runner's `before` hook does; session creation alone does not establish readiness. |
| Quit test fails immediately although the app exited | PowerShell's `Get-Process -ErrorAction SilentlyContinue` can leave an unsuccessful exit status for an already-gone PID. Retain the explicit successful `exit 0` after the guarded wait; a live process exceeding the timeout must still fail. |
| Python notice generation fails on Windows | Use explicit UTF-8 for text I/O. CI runs notice generation with `-X warn_default_encoding -W error::EncodingWarning`. |
| Packaging rejects a dirty tree after a successful build | Read the reported paths and inspect the diff. Tauri has normalized dependency-array spacing in `Cargo.toml`; commit the stable formatting. Do not blindly reset changes or weaken the clean-tree guard. |
| Artifacts under `.tmp/` are missing in CI | Uploads of ignored scratch outputs require the workflow's `include-hidden-files: true` and explicit paths. Never broaden the upload to the whole workspace or `.tmp/`. |
| Expected compile failure breaks the PowerShell step incorrectly | Follow the release guard in the workflow: capture native output/exit code, verify the specific guard message, and succeed only for that expected failure. |
| Browser preview works but installed app fails | Verify bundle resources, native command behavior, WebView2, and actual executable identity. Frontend mocks do not validate native capture/storage/clipboard. |

For a normal Stop regression, verify capture ends, audio/history is admitted, and processing proceeds. Cancel-only smoke tests once missed a Windows file-handle access failure on Stop. For recovery regressions, include partially written journals and later-stage timeouts after asynchronous transcription; assert retry/resume semantics and clearing of stale errors.

## Iterate efficiently with CI

Feature changes run through `pull_request`; automatic `push` builds are limited to `main` to avoid building the same feature revision twice. Branches without a PR can use `workflow_dispatch`. The glass-recorder suite creates independent recordings per scenario and checks either animated or static rendering according to the desktop's reduced-motion preference. Its synthetic screenshots and rendering environment are uploaded with desktop evidence.

The [Windows workflow](../.github/workflows/windows.yml) separates media compilation, Windows checks/build/packaging, desktop acceptance, and signed release publication. Read the failed step's logs before changing source. A cold native release build can take many minutes; waiting alone is not evidence that it is stuck.

- For a transient desktop failure on unchanged source, rerun the failed job using retained artifacts instead of repeating compilation.
- For a test-harness change, manually run the workflow on the new test revision with `acceptance_run_id` set to the earlier build run. Both `x-talk-windows-x64` and `acceptance-build` artifacts must still be available. This diagnoses current tests against older binaries; it does not validate application changes in the new revision.
- Artifact-reuse runs cannot create releases. The final main build must complete the full workflow. Do not infer that the source revision of the test harness is the revision compiled into reused binaries.
- If an explicitly requested diagnostic dispatch duplicates a branch build, cancel only the identified redundant run after confirming which run is needed. Do not cancel another contributor's work or an intended release run.
- Keep screenshots and logs scoped to synthetic test data. Record the run URL and tested commit, not raw API responses containing credentials.
- For documentation-only changes on top of a fully verified revision, check the docs and confirm the application diff is empty. Local application rebuilds are unnecessary, but the main-branch ruleset still requires hosted PR checks. Do not use a CI skip marker with required checks; it leaves the pull request blocked.

## Finish and hand off

Review the final diff and list exactly what changed, what passed, and what remains untested. Distinguish offline tests, real hardware acceptance, live provider checks, and installer tests. Overall owner acceptance is not a claim that every device/provider/upgrade combination was measured.

When release work is requested, follow [RELEASING.md](RELEASING.md): version consistency, clean source, corresponding dependency sources/notices, checksums, and tests are part of the deliverable. Do not overwrite published assets or move published tags. Main pushes now automatically publish after checks pass: treat a main push as deployment and follow the authorization in the current task. Repository visibility changes, signing enrollment, and paid-service setup remain separate decisions.

Leave temporary plans, fixtures, screenshots and logs under `.tmp/`. Do not commit workstation authentication, agent permission overrides, API keys, or personal recordings. Keep this guide updated when an actual workflow changes, rather than adding another overlapping handoff document.

Updater acceptance uses `XTALK_E2E_UPDATES=1` with the isolated 0.0.0 acceptance build. The runner serves loopback fixtures for unavailable/current/malformed feeds, confirmation, and invalid signatures. Only disposable hosted CI sets `XTALK_UPDATE_INSTALL=1` to hand the signed installer to NSIS, verify restart, and recheck preserved production data. Release builds never accept the fixture endpoint environment variable.
