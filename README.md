# x-talk

For when the thought is there, but typing it all out... isn't happening.

x-talk is a small dictation app for Windows and Mac. Hit a shortcut, say what's on your mind, and paste the text wherever you need it. Want a little cleanup first? An optional AI pass can tidy up the rambling. Or leave it in. Your call.

I built this because I wanted a record button, my words, and a little help making them readable. That's pretty much it!

**Windows x64 · macOS 14.2+ on Apple Silicon (new) · English speech · Open source**

[Download](https://github.com/alexander-north/x-talk/releases) · [Report a bug](https://github.com/alexander-north/x-talk/issues) · [Contribute](CONTRIBUTING.md)

![x-talk recorder in dark mode with a glowing microphone and transcript cards](docs/images/recorder.png)

*Dark theme shown with custom shortcuts.*

## Give it a try

You'll need an API key from a supported provider. There's no x-talk account or app subscription; you pay your providers for what you use.

1. Grab the download for your computer from [Releases](https://github.com/alexander-north/x-talk/releases):
   - **Windows:** `x-talk_<version>_x64-setup.exe`. Install it; the installer can add Microsoft WebView2 if needed.
   - **Mac (Apple Silicon, macOS 14.2 or later):** `x-talk_<version>_aarch64.dmg`. Open it and drag x-talk into **Applications**. The first launch needs one extra approval, described below.
2. Open **Settings → API Keys**, add your key, then pick a transcription provider and model in **Providers**.
3. Press **Ctrl+Alt+Space** (on a Mac, **Control+Option+Space**), talk, and press it again to stop. The microphone button works too.
4. Paste! With automatic copying enabled, your transcript goes straight to the clipboard. You can also copy it from the app.

For cleanup, enable **Transformation** and choose its provider, model, and prompts. The cleaned-up text replaces the transcript on your clipboard when ready; both stay available in the app. Automatic copying is configurable in **Delivery and Sounds**.

**Ctrl+Alt+Escape** (**Control+Option+Escape** on a Mac) cancels. Canceling a recording discards it; canceling processing keeps saved audio and text. Requests already sent to a provider may still incur charges.

<details>
<summary>A few things to know before installing</summary>

The Windows installer isn't publisher-signed yet, so you may see an unknown-publisher or SmartScreen warning. Updater signatures are separate. Don't disable Windows security protections.

Tested on Windows 10 Pro x64; Windows 11 hasn't had a separate hardware acceptance pass.

**Mac support is new.** The Mac build is compiled and packaged by CI on Apple Silicon, but it hasn't had a hands-on acceptance pass yet: recording, meeting capture with system audio, shortcuts, the menu bar icon, start at login, Keychain storage, and in-app updates are untested on real hardware. Please [report](https://github.com/alexander-north/x-talk/issues) anything that doesn't work. Intel Macs aren't supported.

The Mac app isn't signed with an Apple Developer ID or notarized, so macOS blocks the first launch. To open it:

1. Open x-talk from **Applications**. macOS says it can't verify the app; click **Done** (not Move to Trash).
2. Open **System Settings → Privacy & Security**, scroll to **Security**, and click **Open Anyway** next to the x-talk message. Confirm with your password or Touch ID.
3. Click **Open** in the dialog that follows. Later launches open normally.

Only do this for a download from this project's Releases page, and check it against `SHA256SUMS.txt` first. Don't turn off Gatekeeper or other macOS security protections.

On a Mac, x-talk asks for **Microphone** access the first time you record; allow it, then start recording again. It asks for **system audio recording** the first time a meeting records system audio. If you declined, turn access back on in **System Settings → Privacy & Security** (Microphone, or Screen & System Audio Recording on macOS 15 and later) and restart x-talk. Because the app is only ad-hoc signed, macOS may ask again for these permissions, and for Keychain access to your saved keys, after an update.

</details>

## A few nice things

- **Less retyping.** Import existing audio, or find an earlier recording to play, edit, export, or retry.
- **Words as you go.** ElevenLabs has an optional real-time mode with a live preview. Imports and retries use batch transcription.
- **Meetings.** Record microphone and system audio, or import audio/video. AssemblyAI creates a speaker-labelled transcript and OpenAI Luna writes the brief. Rename or merge speakers, correct turns, and copy the complete meeting as JSON. Video imports retain only extracted audio. Meeting audio defaults to the latest 50 recordings; text stays until you delete it.
- **A little room on your screen.** Compact mode keeps just the recording controls. Pick a light, dark, or system theme, and adjust sounds and shortcuts to suit you.

Closing the window keeps x-talk in the tray (the menu bar on a Mac). Use **Quit** to exit, or change that behavior in **Settings → General**. Updates live there too. Versions before 1.5 need one manual upgrade; in-app downloads require public GitHub releases.

If the start sound sneaks into your recording, try headphones or turn that cue off in **Delivery and Sounds**. There's no echo cancellation yet.

## Your keys, your data

**Transcription:** OpenAI, ElevenLabs, AssemblyAI, or Deepgram. **Cleanup:** OpenAI or Anthropic. OpenAI, ElevenLabs, and the AssemblyAI/Luna meeting workflow have had live integration checks; other adapters still need live acceptance.

- Audio goes to your chosen transcription provider. Cleanup sends your transcript and configured prompts/context to its provider. Their pricing, privacy, and retention policies apply.
- Keys stay in **Windows Credential Manager**, or in your **login keychain** on a Mac (as passwords for `com.xander.xtalk`). The app doesn't load `.env` files.
- Recordings and text stay under `%LOCALAPPDATA%\com.xander.xtalk` on Windows and `~/Library/Application Support/com.xander.xtalk` on a Mac. Audio retention is configurable. Deleting local recordings doesn't delete copies already sent to providers.
- No built-in analytics or diagnostic uploads. Diagnostic logs are local, on by default, and exclude credentials, audio, transcript text, and filenames. Find or disable them in General. Review logs before sharing them.

## Found something weird?

I'd love to hear about it. [Open an issue](https://github.com/alexander-north/x-talk/issues), tell me what happened, and what you expected instead. Something feeling confusing is useful feedback too. You don't need to write code to help.

Small fixes and ideas are welcome. For bigger changes, let's talk in an issue first. See [Contributing](CONTRIBUTING.md) for details, or [Security](SECURITY.md) to report a vulnerability privately.

## Build and test

<details>
<summary>Here to tinker? Start here.</summary>

**Windows:** use Node.js 24.11.1, Rust 1.93.1, Visual Studio 2022 C++ Build Tools/Windows SDK, Python 3.11+, and WebView2. Keep the Windows checkout on a Windows drive, such as `C:\dev\x-talk`.

Prepare the bundled media tools in WSL/Linux with Python, curl, make, pkg-config, and x86_64 MinGW GCC/G++ installed:

```sh
scripts/build-media.sh
```

With separate checkouts, copy the generated `src-tauri/binaries/ffmpeg.exe` and `ffprobe.exe` into the Windows checkout's same directory. Keep platform dependencies and build caches separate; see [Windows/WSL setup](docs/AGENT_WORKFLOW.md#wsl-and-windows-build-workflow).

From the Windows checkout:

```sh
npm ci
python scripts/generate-notices.py
npm run check
npm test
python -m unittest discover -s tests/release -v
cargo test --manifest-path src-tauri/Cargo.toml --features desktop --locked
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop,test-providers --all-targets --locked -- -D warnings
npm run desktop:dev
```

- `npm run desktop:ui`: native UI development with a separate persistent profile and simulated providers. The microphone, playback, and clipboard are real. See [UI development](docs/AGENT_WORKFLOW.md#native-ui-iteration).
- `npm run desktop:debug`: debug executable with the UI embedded.
- `npm run desktop:build`: Windows installer in `src-tauri/target/release/bundle/nsis/`, or on a Mac the app and disk image in `src-tauri/target/release/bundle/macos/` and `bundle/dmg/`.
- Linux core tests: `cargo test --manifest-path src-tauri/Cargo.toml --locked`, with system FFmpeg installed.

**Mac (Apple Silicon, macOS 14.2+):** install the Xcode Command Line Tools (`xcode-select --install`), Rust 1.93.1 via rustup, Node.js 24.11.1, Python 3.11+, and [Homebrew](https://brew.sh) FFmpeg for development:

```sh
brew install ffmpeg
touch src-tauri/binaries/ffmpeg src-tauri/binaries/ffprobe
npm ci
python3 scripts/generate-notices.py
npm run check
npm test
cargo test --manifest-path src-tauri/Cargo.toml --features desktop --locked
npm run desktop:dev
```

The Tauri build needs files at the bundled tool paths; debug builds treat these empty placeholders as missing and use Homebrew's `ffmpeg`/`ffprobe` instead. `npm run desktop:build` and `desktop:debug` instead verify the pinned bundled tools, which `scripts/build-media.sh` builds natively on an Apple Silicon Mac; see [Releasing](docs/RELEASING.md). Desktop end-to-end tests run on Windows only.

For isolated desktop acceptance without microphone or provider requests, run in PowerShell on Windows:

```powershell
cargo install tauri-driver --locked --version 2.0.5
./scripts/install-webdriver.ps1
npm run desktop:test-build
$env:XTALK_E2E_OFFLINE = '1'
npm run test:e2e
Remove-Item Env:XTALK_E2E_OFFLINE
```

The setup script downloads the driver matching WebView2. For real microphone, live provider, and installer tests, read [test modes and isolation](docs/AGENT_WORKFLOW.md#native-test-modes-and-isolation) first.

[Product behavior](SPEC.md) · [Agent workflow](docs/AGENT_WORKFLOW.md) · [Releasing](docs/RELEASING.md)

</details>

## License

[Apache 2.0](LICENSE) for original code, prompts, artwork, docs, and synthesized sounds, unless noted otherwise. Third-party dependencies keep their own licenses. See [NOTICE](NOTICE) for attribution and [Releasing](docs/RELEASING.md) for bundled notices and source archives. The license doesn't grant trademark rights or imply endorsement of forks.
