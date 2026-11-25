"""rgp_parse.py <file.rgp|dir>... [--json out.json] : headless reader for Mesa/RADV .rgp captures.
Walks the SQTT file chunks (Mesa ac_rgp.h layout: 56-byte file header, 16-byte chunk headers) and decodes:
- QUEUE_EVENT_TIMINGS (type 5): queue info records + 56-byte event records (CMDBUF_SUBMIT with GPU start/end ticks, semaphores, PRESENT);
- CLOCK_CALIBRATION (type 6); ASIC_INFO (type 0) for the GPU name and timestamp frequency (searched, validated);
- SQTT_DATA (type 2) sizes per shader engine (the token stream itself is not decoded: no open decoder).
Per capture it prints: submits, command buffers, GPU busy (union of [start,end]) vs first-start..last-end span, gaps, per-queue totals,
and the largest command buffers. Command-buffer GPU spans are per VkCommandBuffer; vkd3d-proton records one D3D12 command list per
VkCommandBuffer (plus its own init/transition buffers), so our wrapper's extra lists show up as extra buffers."""
import json, os, struct, sys
from collections import defaultdict

TYPES = {0: 'ASIC_INFO', 1: 'SQTT_DESC', 2: 'SQTT_DATA', 3: 'API_INFO', 4: 'RESERVED', 5: 'QUEUE_EVENT_TIMINGS', 6: 'CLOCK_CALIBRATION',
         7: 'CPU_INFO', 8: 'SPM_DB', 9: 'CODE_OBJECT_DATABASE', 10: 'CODE_OBJECT_LOADER_EVENTS', 11: 'PSO_CORRELATION', 12: 'INSTRUCTION_TABLE'}
EV = {0: 'CMDBUF_SUBMIT', 1: 'SIGNAL_SEMAPHORE', 2: 'WAIT_SEMAPHORE', 3: 'PRESENT'}


def chunks(b):
    magic, = struct.unpack_from('<I', b, 0)
    if magic != 0x50303042:
        raise ValueError('not an RGP/SQTT file (magic %08x)' % magic)
    off = struct.unpack_from('<I', b, 16)[0] if len(b) > 20 else 56
    if not 16 <= off <= 4096:
        off = 56
    while off + 16 <= len(b):
        cid, minor, major, size, _pad = struct.unpack_from('<IHHii', b, off)
        if size <= 0 or off + size > len(b):
            break
        yield cid & 0xff, (cid >> 8) & 0xff, minor, major, off, size
        off += size


def ts_freq(b, off, size):
    # Mesa sqtt_file_chunk_asic_info (v0.5, 768 bytes, checked on an RDNA 3.5 RADV capture): gpu_name[256] at chunk offset 152,
    # then 4 floats, then gpu_timestamp_frequency (u64) at 424 (99.81 MHz there).
    blob = b[off:off + size]
    name = blob[152:152 + 256].split(b'\0')[0].decode('ascii', 'replace') if size >= 432 else '?'
    cands = []
    if size >= 432:
        f, = struct.unpack_from('<Q', blob, 424)
        if 1_000_000 <= f <= 2_000_000_000:
            cands.append((424, f))
    return name, cands


def parse(path):
    b = open(path, 'rb').read()
    out = {'file': os.path.basename(path), 'bytes': len(b), 'chunks': defaultdict(int), 'sqtt_data_bytes': 0}
    qinfo, events, calib, freq, name = [], [], [], None, '?'
    for typ, idx, minor, major, off, size in chunks(b):
        out['chunks'][TYPES.get(typ, str(typ))] += 1
        if typ == 0:
            name, c = ts_freq(b, off, size)
            if c:
                freq = c[0][1]
        elif typ == 2 and size >= 24:
            o, s = struct.unpack_from('<ii', b, off + 16); out['sqtt_data_bytes'] += max(s, 0)
        elif typ == 6 and size >= 40:
            calib.append(struct.unpack_from('<QQ', b, off + 16))
        elif typ == 5 and size >= 32:
            nqi, qis, nev, evs = struct.unpack_from('<IIII', b, off + 16)
            # the two size fields are TOTAL bytes (one checked capture: 5 info records = 120 bytes, 342 events = 19152 bytes)
            qis = qis // nqi if nqi else 24; evs = evs // nev if nev else 56
            p = off + 32
            for k in range(nqi):
                qid, qctx = struct.unpack_from('<QQ', b, p); hw, = struct.unpack_from('<I', b, p + 16)
                qinfo.append(dict(queue_id=qid, ctx=qctx, queue_type=hw & 0xff, engine_type=(hw >> 8) & 0xff)); p += qis
            for k in range(nev):
                et, cb, fidx, qi, sub, api, cpu, g0, g1 = struct.unpack_from('<IIQIIQQQQ', b, p)
                events.append(dict(type=EV.get(et, et), cb=cb, frame=fidx, queue=qi, sub=sub, api=api, cpu=cpu, g0=g0, g1=g1)); p += evs
    out['chunks'] = dict(out['chunks']); out['gpu'] = name; out['freq'] = freq; out['queues'] = qinfo; out['calib'] = calib
    subs = [e for e in events if e['type'] == 'CMDBUF_SUBMIT' and e['g1'] > e['g0'] > 0]
    out['events'] = len(events); out['submits'] = len(subs)
    out['event_types'] = {t: sum(1 for e in events if e['type'] == t) for t in set(e['type'] for e in events)}
    if subs and freq:
        ms = 1000.0 / freq
        t0 = min(e['g0'] for e in subs); t1 = max(e['g1'] for e in subs)
        iv = sorted((e['g0'], e['g1']) for e in subs); busy = 0; cur = None
        for a, c in iv:
            if cur is None or a > cur[1]:
                if cur: busy += cur[1] - cur[0]
                cur = [a, c]
            else:
                cur[1] = max(cur[1], c)
        busy += cur[1] - cur[0]
        perq = defaultdict(lambda: [0, 0.0])
        for e in subs:
            perq[e['queue']][0] += 1; perq[e['queue']][1] += (e['g1'] - e['g0']) * ms
        out['span_ms'] = (t1 - t0) * ms; out['busy_ms'] = busy * ms; out['sum_cb_ms'] = sum((e['g1'] - e['g0']) for e in subs) * ms
        out['per_queue'] = {str(q): dict(cbs=v[0], ms=round(v[1], 3)) for q, v in sorted(perq.items())}
        cbs = sorted(subs, key=lambda e: e['g1'] - e['g0'], reverse=True)
        out['top'] = [dict(ms=round((e['g1'] - e['g0']) * ms, 3), start_ms=round((e['g0'] - t0) * ms, 3), queue=e['queue'], cb=e['cb'], sub=e['sub']) for e in cbs[:15]]
        durs = sorted((e['g1'] - e['g0']) * ms for e in subs)
        out['cb_ms_hist'] = {k: sum(1 for d in durs if lo <= d < hi) for k, lo, hi in (('<0.05', 0, .05), ('0.05-0.2', .05, .2), ('0.2-1', .2, 1), ('1-5', 1, 5), ('>=5', 5, 1e9))}
        out['timeline'] = [dict(s=round((e['g0'] - t0) * ms, 3), e=round((e['g1'] - t0) * ms, 3), q=e['queue'], sub=e['sub'], api=e['api']) for e in sorted(subs, key=lambda e: e['g0'])]
        out['frames'] = sorted(set(e['frame'] for e in subs)); out['distinct_api'] = len(set(e['api'] for e in subs))
        pres = sorted(e['g0'] for e in events if e['type'] == 'PRESENT' and e['g0'])
        out['present_gpu_ms'] = [round((p - t0) * ms, 3) for p in pres]
    return out


def main():
    args = sys.argv[1:]; js = None
    if '--json' in args:
        i = args.index('--json'); js = args[i + 1]; del args[i:i + 2]
    files = []
    for a in args:
        if os.path.isdir(a):
            files += sorted(os.path.join(a, f) for f in os.listdir(a) if f.endswith('.rgp'))
        else:
            files.append(a)
    res = []
    for f in files:
        try:
            r = parse(f)
        except Exception as e:
            print('%s: %s' % (f, e)); continue
        res.append(r)
        print('%s %.1f MB gpu=%s freq=%s chunks=%s sqtt=%.1f MB events=%d %s' % (r['file'], r['bytes'] / 1e6, r['gpu'], r['freq'], r['chunks'], r['sqtt_data_bytes'] / 1e6, r['events'], r['event_types']))
        if 'span_ms' in r:
            print('   submits(cb)=%d span=%.2f ms busy=%.2f ms sum_cb=%.2f ms per_queue=%s hist=%s' % (r['submits'], r['span_ms'], r['busy_ms'], r['sum_cb_ms'], r['per_queue'], r['cb_ms_hist']))
            print('   top: ' + ', '.join('%.2f@%.1f(q%d)' % (t['ms'], t['start_ms'], t['queue']) for t in r['top'][:10]))
            print('   frames=%s distinct_api=%d presents_at_ms=%s queues=%s' % (r['frames'], r['distinct_api'], r['present_gpu_ms'], [(q['queue_type'], q['engine_type']) for q in r['queues']]))
    if js:
        json.dump(res, open(js, 'w'), indent=1)


if __name__ == '__main__':
    main()
