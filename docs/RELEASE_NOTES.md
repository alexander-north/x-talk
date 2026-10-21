# Meeting workspace refinements

Returning to Meetings now opens the library, with Record and Import actions together in its header. Speaker editing has cleaner spacing, save confirmations and action errors use gold toasts, and animated indicators show processing and active recording. Meeting audio uses the existing play, pause, and seek controls.

Meetings now sits directly below Recorder. Import through a file picker or drop zone, set a title and recording date, and see a processing card immediately. Background imports stay visible across navigation, including failures. Meeting audio retention lives in Settings → Meetings. Device selectors show fuller Windows names, recording controls collapse again, and brief citations are constrained to valid transcript evidence. Existing failed briefs can be regenerated without re-transcribing.

Record a meeting using your microphone and system audio, or import an audio or video file. Video imports save only the extracted audio. AssemblyAI provides speaker-labelled transcripts and OpenAI Luna produces the meeting brief. Add both provider keys in Settings.

Review briefs and timestamped transcripts, name or merge speakers, correct individual turns, and copy the complete meeting as JSON. Download retained audio when needed. Live meeting audio is compressed locally to keep retained recordings manageable. Meeting audio retention is separate from dictation and defaults to the latest 50 recordings; transcripts and briefs remain until explicitly deleted.

Meeting recording supports Start and Stop, source muting, compact controls, and tray access. Use headphones to avoid duplicate speaker pickup; echo cancellation is not included. Existing dictation remains available.

Download the `x-talk_<version>_x64-setup.exe` asset for Windows x64. Add your own provider key under Settings → API Keys; provider usage is billed separately. Installers and signed updates are available from the public GitHub releases.

**The Windows publisher is still unsigned.** Updater signatures protect updates but do not remove unknown-publisher or SmartScreen warnings. WebView2 is required; the installer can download it if missing. Do not disable Windows security protections.

History and diagnostics stay local. Audio goes to your chosen speech provider; transformation sends text to its provider. macOS is not supported. Microphone, shortcut, and speaker behavior varies by device. Use headphones or disable start cues to avoid speaker pickup. OpenAI, ElevenLabs, and the AssemblyAI/Luna meeting workflow have live checks; other providers still need live acceptance.

Original x-talk material is Apache-2.0. The release includes dependency notices/sources and the exact source/build recipe for its separate LGPL FFmpeg executables. Verify downloads against SHA256SUMS.txt.
