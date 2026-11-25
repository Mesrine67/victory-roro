#!/usr/bin/env python3
"""frame_classes.py: classify slow frames the way the write-up describes (streaming / submission burst / zero-need), with
harness capture frames excluded, and print per-minute rates per class.

Input: a CSV with one row per presented frame and these columns (header required; extra columns are ignored):
  t_s           frame end time in seconds (any monotonic clock)
  frame_ms      present-to-present interval
  submissions   ExecuteCommandLists calls in the frame (direct queue, or all queues; be consistent)
  needs         spans/textures your submit-time barrier had to copy in this frame (0 if none)
  capture       1 if your own harness captured a screenshot/sample in this frame, else 0
  stretch       optional label; only rows whose stretch matches --stretch are used (e.g. "race", "lap")

usage: frame_classes.py frames.csv [--line 33.4] [--burst 30] [--stretch race] [--after-capture 0|1] [--burst-window 10]

Why these classes: in our measurements, slow frames with barrier needs (streaming) were about two-thirds of real slow frames and
were the only class head-start staging could improve; frames with a burst of submissions and zero needs were a separate class
that came 1-10 frames BEFORE most heavy streaming frames. Always compare arms by class, with capture frames excluded.
"""
import csv, sys
from collections import Counter


def arg(name, default, cast=float):
    a = sys.argv
    return cast(a[a.index(name) + 1]) if name in a else default


def main():
    if len(sys.argv) < 2 or sys.argv[1].startswith('-'):
        print(__doc__); sys.exit(2)
    line = arg('--line', 33.4); burst = arg('--burst', 30, int); stretch = arg('--stretch', None, str)
    drop_after = arg('--after-capture', 0, int); window = arg('--burst-window', 10, int)
    rows = []
    with open(sys.argv[1], newline='') as f:
        for r in csv.DictReader(f):
            if stretch and r.get('stretch', '') != stretch:
                continue
            rows.append(dict(t=float(r['t_s']), ms=float(r['frame_ms']), sub=int(float(r['submissions'])),
                             need=int(float(r['needs'])), cap=int(float(r.get('capture', 0) or 0))))
    if not rows:
        print('no rows'); return
    minutes = max((rows[-1]['t'] - rows[0]['t']) / 60.0, 1e-9)
    excluded = set()
    for i, r in enumerate(rows):
        if r['cap']:
            excluded.add(i)
            if drop_after and i + 1 < len(rows):
                excluded.add(i + 1)
    cls = Counter(); last_burst = -10**9; stream_after_burst = 0
    for i, r in enumerate(rows):
        is_burst = r['sub'] >= burst and r['need'] == 0
        if is_burst:
            last_burst = i
        if r['ms'] <= line:
            continue
        if i in excluded:
            cls['capture (excluded)'] += 1; continue
        if r['need'] >= 1:
            cls['streaming'] += 1
            if 0 < i - last_burst <= window:
                stream_after_burst += 1
        elif is_burst:
            cls['burst'] += 1
        else:
            cls['zero-need'] += 1
    real = cls['streaming'] + cls['burst'] + cls['zero-need']
    print('frames=%d  minutes=%.2f  line=%.1f ms  burst>=%d submissions' % (len(rows), minutes, line, burst))
    print('real slow frames: %d (%.1f/min)' % (real, real / minutes))
    for k in ('streaming', 'burst', 'zero-need', 'capture (excluded)'):
        print('  %-20s %6d  %6.1f/min' % (k, cls[k], cls[k] / minutes))
    if cls['streaming']:
        print('streaming slow frames within %d frames after a burst: %d (%.0f%%)' % (window, stream_after_burst, 100.0 * stream_after_burst / cls['streaming']))


if __name__ == '__main__':
    main()
