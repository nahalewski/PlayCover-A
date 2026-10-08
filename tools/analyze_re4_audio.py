"""Inspect opt-in RE4 output capture without recording device microphones."""
import argparse
import csv
import io
import json
from pathlib import Path
import re
import wave
import numpy as np


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', default='pixel-fold-tests/re4-audio-baseline')
    args = parser.parse_args()
    folder = Path(args.directory)
    timing = (folder / 're4-audio-timing.csv').read_text()
    header, rows = timing.split('\n', 1)
    rate = float(re.search(r'playback_rate=([0-9.]+)', header).group(1))
    stride = int(re.search(r'bytes_per_frame: ([0-9]+)', header).group(1))
    channels = stride // 2  # decode_buffer derives channels from actual stride.
    if channels not in (1, 2):
        raise ValueError('Capture must contain 16-bit mono or stereo PCM')
    records = list(csv.DictReader(io.StringIO(rows)))
    payload = (folder / 're4-audio-output.pcm').read_bytes()
    payload = payload[:len(payload) - len(payload) % stride]
    def statistics(order):
        values = np.frombuffer(payload, dtype=order + 'i2').reshape(-1, channels).astype(np.float64)
        active = values[np.any(values != 0, axis=1)]
        delta = np.diff(active, axis=0)
        return dict(frames=len(values), active_frames=len(active),
                    rms=float(np.sqrt(np.mean(active ** 2))) if len(active) else 0,
                    clipped_fraction=float(np.mean(np.abs(active) >= 32760)) if len(active) else 0,
                    adjacent_difference_rms=float(np.sqrt(np.mean(delta ** 2))) if len(delta) else 0,
                    same_channels_fraction=(float(np.mean(active[:, 0] == active[:, 1]))
                                            if len(active) and channels == 2 else None))
    elapsed = sum(float(row['elapsed_seconds']) for row in records)
    generated = sum(int(row['frames']) for row in records) / rate
    report = dict(format=header, playback_rate=rate, effective_channels=channels,
                  little_endian=statistics('<'),
                  big_endian=statistics('>'), callback_count=len(records),
                  elapsed_seconds=elapsed, generated_seconds=generated,
                  generation_to_elapsed_ratio=generated / elapsed if elapsed else None,
                  capped_callback_count=sum(int(row['frames']) == 2048 for row in records),
                  slow_callback_count=sum(float(row['elapsed_seconds']) > 2048 / rate for row in records))
    with wave.open(str(folder / 're4-output.wav'), 'wb') as output:
        output.setnchannels(channels)
        output.setsampwidth(2)
        output.setframerate(round(rate))
        output.writeframes(payload)
    (folder / 'analysis.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
