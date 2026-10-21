# One recorder, two modes

Switch between Dictation and Meeting on the recorder. Meeting capture uses a pink-and-gold orb, device selection, live levels and microphone/system mute controls. Transcripts and Meetings now have separate library icons below the navigation divider.

Meeting brief and transcript cards scroll internally, keeping the full-width audio player and review tabs visible. A pencil beside the speaker name opens turn editing; Save applies text, speaker assignment and name changes together.

Compatible imports skip unnecessary audio encoding. Imports create processing cards immediately and continue across navigation. Settings displays the full version and source revision. Meeting retention, gold save/error toasts and animated processing indicators remain available.

Record a meeting using your microphone and system audio, or import an audio or video file. Video imports save only the extracted audio. AssemblyAI provides speaker-labelled transcripts and OpenAI Luna produces the meeting brief. Add both provider keys in Settings.

Review briefs and timestamped transcripts, name or merge speakers, correct individual turns, and copy the complete meeting as JSON. Download retained audio when needed. Live meeting audio is compressed locally to keep retained recordings manageable. Meeting audio retention is separate from dictation and defaults to the latest 50 recordings; transcripts and briefs remain until explicitly deleted.

Meeting recording supports Start and Stop, source muting, compact controls, and tray access. Use headphones to avoid duplicate speaker pickup; echo cancellation is not included. Existing dictation remains available.

Download the `x-talk_<version>_x64-setup.exe` asset for Windows x64. Add your own provider key under Settings → API Keys; provider usage is billed separately. Installers and signed updates are available from the public GitHub releases.

**The Windows publisher is still unsigned.** Updater signatures protect updates but do not remove unknown-publisher or SmartScreen warnings. WebView2 is required; the installer can download it if missing. Do not disable Windows security protections.

History and diagnostics stay local. Audio goes to your chosen speech provider; transformation sends text to its provider. macOS is not supported. Microphone, shortcut, and speaker behavior varies by device. Use headphones or disable start cues to avoid speaker pickup. OpenAI, ElevenLabs, and the AssemblyAI/Luna meeting workflow have live checks; other providers still need live acceptance.

Original x-talk material is Apache-2.0. The release includes dependency notices/sources and the exact source/build recipe for its separate LGPL FFmpeg executables. Verify downloads against SHA256SUMS.txt.
