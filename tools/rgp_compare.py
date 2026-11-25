"""rgp_compare.py <label>=<dir with .rgp files> ... : per-capture and median table (markdown) of the per-command-buffer GPU data from
rgp_parse.py: command buffers per frame by GPU-time class, GPU busy (union), span first-start..last-end, idle inside the span, the GPU time
of tiny (<0.05 ms) and big (>=1 ms) buffers, and the largest buffers. One capture = one frame (MESA_VK_TRACE trigger)."""
import os, statistics as st, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rgp_parse import parse

CLS = (('tiny<0.05', 0, .05), ('0.05-0.2', .05, .2), ('0.2-1', .2, 1), ('>=1', 1, 1e9))


def stats(r):
    tl = r.get('timeline', [])
    d = [t['e'] - t['s'] for t in tl]
    row = dict(file=r['file'], cbs=len(d), span=r.get('span_ms', 0), busy=r.get('busy_ms', 0))
    row['idle'] = row['span'] - row['busy']
    for name, lo, hi in CLS:
        row['n ' + name] = sum(1 for x in d if lo <= x < hi); row['ms ' + name] = sum(x for x in d if lo <= x < hi)
    row['top3'] = ' '.join('%.2f' % x for x in sorted(d, reverse=True)[:3])
    return row


def main():
    cols = ['cbs', 'span', 'busy', 'idle'] + [p + n for n, _, _ in CLS for p in ('n ', 'ms ')]
    print('| run | capture | ' + ' | '.join(cols) + ' | top3 ms |'); print('|' + '---|' * (len(cols) + 3))
    med = {}
    for arg in sys.argv[1:]:
        label, d = arg.split('=', 1)
        rows = []
        for f in sorted(os.listdir(d)):
            if f.endswith('.rgp'):
                try:
                    rows.append(stats(parse(os.path.join(d, f))))
                except Exception as e:
                    print('| %s | %s | parse error %s |' % (label, f, e))
        for r in rows:
            print('| %s | %s | ' % (label, r['file'].split('_', 1)[-1][-22:-4]) + ' | '.join(('%.2f' % r[c]) if isinstance(r[c], float) else str(r[c]) for c in cols) + ' | %s |' % r['top3'])
        if rows:
            med[label] = {c: st.median(r[c] for r in rows) for c in cols}; med[label]['n'] = len(rows)
    print('\n| run (median over captures) | n | ' + ' | '.join(cols) + ' |'); print('|' + '---|' * (len(cols) + 2))
    for label, m in med.items():
        print('| %s | %d | ' % (label, m['n']) + ' | '.join('%.2f' % m[c] for c in cols) + ' |')


if __name__ == '__main__':
    main()
