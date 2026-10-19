"""Seed non-sensitive saved history into a disposable CI profile while the app is stopped."""
import json
import os
from pathlib import Path
import sqlite3
import wave

if os.environ.get('GITHUB_ACTIONS') != 'true':
    raise SystemExit('This fixture is restricted to disposable GitHub Actions profiles')
root = Path(os.environ['LOCALAPPDATA']) / 'com.xander.xtalk'
identifier = '00000000-0000-4000-8000-000000000141'
audio = root / 'audio' / f'{identifier}.wav'
with wave.open(str(audio), 'wb') as writer:
    writer.setnchannels(1)
    writer.setsampwidth(2)
    writer.setframerate(24000)
    writer.writeframes(b'\0\0' * 24000)
entry = dict(id=identifier, created='2026-10-17T00:00:00Z', source='import',
             title='Installer retention fixture', subtitle='', subtitle_manual=False,
             revision=1, audio=str(audio), duration=1.0, bytes=audio.stat().st_size,
             transcript='Saved raw text before upgrade.', transcript_revision=1,
             transformed='Saved transformed text before upgrade.', transformed_revision=1,
             transcription_ms=50, transformation_ms=100, transcript_model='fixture',
             transform_model='fixture')
with sqlite3.connect(root / 'x-talk.sqlite3') as db:
    db.execute('INSERT INTO entries(id,document) VALUES(?,?)', (identifier, json.dumps(entry)))
print('Disposable upgrade history fixture created.')
