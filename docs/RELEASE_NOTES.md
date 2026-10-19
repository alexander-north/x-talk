# x-talk: live words and a simpler recording workspace

ElevenLabs now offers an optional real-time recording mode. See words appear while you speak, then press Stop to finish the transcript and run any enabled cleanup. After-recording transcription remains the default; imports and retries still use batch transcription. Real-time mode sends microphone audio to ElevenLabs while recording, and provider charges apply.

The recorder has animated glass controls, a live microphone spectrum, and overlapping transcript and cleaned-up text cards. Compact mode keeps the controls close without taking over your screen, and reduced-motion settings use static visuals.

Transcripts now copy to the clipboard as soon as transcription finishes, even with transformation enabled. Paste immediately for the raw text, or wait for transformation to replace it. Each stage has its own copy toggle; imports, retries, and resumed jobs still require manual copying.

This release includes in-app updates and a simpler shortcut editor. You can capture a key combination, enter it manually, or remove it. Toggle recording starts and stops; Cancel discards the active recording.

Under Settings → General, x-talk can check for new versions automatically. You choose when to download, update, and restart. Updates wait until recording and processing finish, verify the installer signature, and preserve your recordings, preferences, and API keys. Older versions need one manual installation to gain the updater.

Download the `x-talk_<version>_x64-setup.exe` asset for Windows x64. Add your own provider key under Settings → API Keys; provider usage is billed separately. Installers and signed updates are available from the public GitHub releases.

**The Windows publisher is still unsigned.** Updater signatures protect updates but do not remove unknown-publisher or SmartScreen warnings. WebView2 is required; the installer can download it if missing. Do not disable Windows security protections.

History and diagnostics stay local. Audio goes to your chosen speech provider; transformation sends text to its provider. macOS is not supported. Microphone, shortcut, and speaker behavior varies by device. Use headphones or disable start cues to avoid speaker pickup. OpenAI and ElevenLabs have prior live checks; other providers still need live acceptance.

Original x-talk material is Apache-2.0. The release includes dependency notices/sources and the exact source/build recipe for its separate LGPL FFmpeg executables. Verify downloads against SHA256SUMS.txt.
