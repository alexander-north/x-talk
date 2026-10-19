#!/usr/bin/env python3
"""Original synthesized UI cues. Run with Python's standard library only.

48 kHz / 16-bit mono playback copies are embedded in the app.
Use --extras to generate 24-bit masters and audition mixes in .tmp/sounds/.
No third-party audio or per-file peak normalization.
"""
import argparse
import math
from pathlib import Path
import wave

RATE = 48_000
ROOT = Path(__file__).resolve().parents[1] / 'assets' / 'sounds'
EVENTS = {
    'started': ('record-start', .400),
    'stopped': ('record-stop', .330),
    'canceled': ('record-cancel', .320),
    'transcribed': ('transcription-complete', .440),
    'transformed': ('polishing-complete', .350),
}


def smooth(x):
    x = min(1., max(0., x))
    return x * x * (3 - 2 * x)


def voice(out, start, length, low, high, gain, attack=.006, decay=3.,
          glide=.065, fm=.3, ratio=2., harmonic=.12):
    """Soft FM pluck with a phase-continuous glide and rapidly damping color.

    Different modulation ratios distinguish hollow gestures from clear result
    plucks. Brightness falls rapidly; the body remains rounded and centered.
    """
    phase = 0.
    for i in range(round(length * RATE)):
        t = i / RATE
        hz = low + (high - low) * smooth(t / glide)
        phase += math.tau * hz / RATE
        color = fm * math.exp(-t / .035)
        body = math.sin(phase + color * math.sin(ratio * phase))
        body += harmonic * math.exp(-t / .045) * math.sin(2 * phase)
        env = smooth(t / attack) * math.exp(-decay * t / length)
        env *= smooth((length - 1 / RATE - t) / .025)
        out[round(start * RATE) + i] += gain * body * env


def render(event):
    out = [0.] * round(EVENTS[event][1] * RATE)
    attack = .006
    if event == 'transformed':
        # The former start gesture, with its final tone raised by 50%.
        for start, length, hz, gain in [
            (0., .105, 392., .42),
            (.075, .110, 494., .46),
            (.150, .200, 988.5, .57),
        ]:
            voice(out, start, length, hz * .98, hz, gain, attack, 2.5,
                  glide=.018, fm=.45, ratio=1., harmonic=.09)
    elif event == 'stopped':
        # Answer with the same rhythm descending to a rounded low closure.
        for start, length, hz, gain in [
            (0., .105, 587., .42),
            (.075, .110, 440., .46),
            (.150, .180, 294., .60),
        ]:
            voice(out, start, length, hz * 1.02, hz, gain, attack, 2.7,
                  glide=.018, fm=.40, ratio=1., harmonic=.17)
    elif event == 'canceled':
        # E3-D3-C3: low 3-2-1, with two short steps and a longer final rest.
        # A little second harmonic keeps the low register audible on laptops.
        for start, length, hz, gain in [
            (0., .085, 164.81, .38),
            (.065, .085, 146.83, .40),
            (.130, .190, 130.81, .48),
        ]:
            voice(out, start, length, hz, hz, gain,
                  .010, 2.6,
                  fm=.18, ratio=1., harmonic=.28)
    else:
        # One shared C-major arc: C4-D4-F4 followed by G4-A4-C5.
        # Both phrases skip upward on the final step (1-2-4 / 5-6-8),
        # with identical timbre and onset spacing so they form one sentence.
        if event == 'started':
            pitches = [261.63, 293.66, 349.23]
            lengths = [.125, .135, .220]
            gains = [.40, .42, .47]
        else:
            pitches = [392., 440., 523.25]
            lengths = [.125, .135, .260]
            gains = [.40, .42, .46]
        for start, length, hz, gain in zip([0., .090, .180], lengths, pitches, gains):
            voice(out, start, length, hz, hz, gain,
                  .010, 2.7,
                  fm=.34, ratio=2., harmonic=.10)
    return out


def write(path, samples, bits):
    path.parent.mkdir(parents=True, exist_ok=True)
    scale = 2 ** (bits - 1) - 1
    payload = b''.join(round(v * scale).to_bytes(bits // 8, 'little', signed=True) for v in samples)
    with wave.open(str(path), 'wb') as target:
        target.setnchannels(1)
        target.setsampwidth(bits // 8)
        target.setframerate(RATE)
        target.writeframes(payload)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--extras', action='store_true',
                        help='Also generate masters and audition mixes in .tmp/sounds/')
    args = parser.parse_args()
    extras = ROOT.parents[1] / '.tmp' / 'sounds'
    sequence = []
    for event, (name, _) in EVENTS.items():
        samples = render(event)
        write(ROOT / 'runtime' / f'{event}.wav', samples, 16)
        if args.extras:
            write(extras / 'masters' / f'{name}.wav', samples, 24)
            sequence.extend(samples)
            sequence.extend([0.] * round(.7 * RATE))
    if args.extras:
        write(extras / 'five-cues.wav', sequence, 16)
        write(extras / 'completion-pair.wav',
              render('transcribed') + render('transformed'), 16)
        write(extras / 'progression.wav',
              render('started') + [0.] * RATE + render('transcribed') + [0.] * RATE + render('transformed'), 16)
    print('Rendered five runtime cues' + (' and scratch audition assets.' if args.extras else '.'))
