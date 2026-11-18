# x-talk: audible failures, faster libraries, safer long recordings

This release fixes long real-time recordings, makes failures audible when x-talk is in the tray, and tightens privacy and the release pipeline.

- **Long real-time recordings keep their transcript.** Recordings past roughly 47 minutes on a stereo microphone no longer fail validation after the transcript was already streamed.
- **You can hear when something goes wrong.** If a shortcut or tray action can't start, or processing fails, x-talk plays a short attention sound and the tray tooltip points you to the window. Turn it off under Delivery and Sounds → Something went wrong.
- **Retry transcription asks first** when it would replace an existing transcript, including your edits.
- **Large libraries stay quick.** Finishing a recording no longer rescans the whole library, and old finished job records are pruned.
- **Clear All clears more.** It also removes old database backups left by upgrades, and deleted text is overwritten in the database.
- **Start at login is off for new installations.** Existing installations keep their setting.
- Missing-key messages now point to Settings → API Keys. The recorder uses less CPU while idle.

After this update the local database cannot be opened by older x-talk versions.

## Also in the 1.6 series

ElevenLabs now offers an optional real-time recording mode. See words appear while you speak, then press Stop to finish the transcript and run any enabled cleanup. After-recording transcription remains the default; imports and retries still use batch transcription. Real-time mode sends microphone audio to ElevenLabs while recording, and provider charges apply.

The recorder has animated glass controls, a live microphone spectrum, and overlapping transcript and cleaned-up text cards. Compact mode keeps the controls close without taking over your screen, and reduced-motion settings use static visuals.

Transcripts now copy to the clipboard as soon as transcription finishes, even with transformation enabled. Paste immediately for the raw text, or wait for transformation to replace it. Each stage has its own copy toggle; imports, retries, and resumed jobs still require manual copying.

This release includes in-app updates and a simpler shortcut editor. You can capture a key combination, enter it manually, or remove it. Toggle recording starts and stops; Cancel discards the active recording.

Under Settings → General, x-talk can check for new versions automatically. You choose when to download, update, and restart. Updates wait until recording and processing finish, verify the installer signature, and preserve your recordings, preferences, and API keys. Older versions need one manual installation to gain the updater.

Download the `x-talk_<version>_x64-setup.exe` asset for Windows x64. Add your own provider key under Settings → API Keys; provider usage is billed separately. Installers and signed updates are available from the public GitHub releases.

**The Windows publisher is still unsigned.** Updater signatures protect updates but do not remove unknown-publisher or SmartScreen warnings. WebView2 is required; the installer can download it if missing. Do not disable Windows security protections.

History and diagnostics stay local. Audio goes to your chosen speech provider; transformation sends text to its provider. macOS is not supported. Microphone, shortcut, and speaker behavior varies by device. Use headphones or disable start cues to avoid speaker pickup. OpenAI and ElevenLabs have prior live checks; other providers still need live acceptance.

Original x-talk material is Apache-2.0. The release includes dependency notices/sources and the exact source/build recipe for its separate LGPL FFmpeg executables. Verify downloads against SHA256SUMS.txt.
