#!/usr/bin/env python3
"""GPU time per frame, and per render pass, from a Metal System Trace.

Frame time on macOS is paced to the display and says little about the GPU; this is what does.
Record while the client benches, then read the trace:

    target/debug/lightcone assets/catalogs/hygdata_v42.csv --rate 0 --bench 1500 &
    sleep 8; xcrun xctrace record --template 'Metal System Trace' --time-limit 3s \\
        --attach $! --output /tmp/run.trace
    python3 tools/gpu_passes.py /tmp/run.trace

Needs Xcode, not only the command-line tools. Passes are named by the labels wgpu gives Bevy's
encoders, so the half-size haze pass and the sky's are both `main_transparent_pass_3d`.
"""
import collections
import subprocess
import sys
import xml.etree.ElementTree as ET

TABLE = '/trace-toc/run[@number="1"]/data/table[@schema="metal-gpu-intervals"]'


def main(trace):
    xml = subprocess.run(['xcrun', 'xctrace', 'export', '--input', trace, '--xpath', TABLE],
                         capture_output=True, text=True, check=True).stdout
    # Repeated values are written once with an `id` and referred to after with `ref`.
    ids = {}

    def resolve(cell):
        return ids[cell.attrib['ref']] if 'ref' in cell.attrib else cell

    per_pass = collections.Counter()
    frames = set()
    for row in ET.fromstring(xml).iter('row'):
        for node in row.iter():
            if 'id' in node.attrib:
                ids[node.attrib['id']] = node
        cells = [resolve(c) for c in row]
        if cells[5].get('fmt') != '0':
            continue  # nested intervals are inside a top-level one already counted
        # The encoder's label is the channel subtitle when there is one, and in the label else.
        subtitle = cells[12].get('fmt') if len(cells) > 12 and cells[12].tag != 'sentinel' else ''
        label = subtitle or cells[6].get('fmt') or ''
        name = label.split(' (')[0].split(':')[0].strip()
        per_pass[f"{cells[2].get('fmt'):9} {name}"] += int(cells[1].text)
        frames.add(cells[3].get('fmt'))

    count = max(1, len(frames))
    print(f'{count} frames, GPU busy {sum(per_pass.values()) / count / 1e6:.2f} ms a frame')
    for name, ns in per_pass.most_common(16):
        print(f'  {ns / count / 1e6:7.3f} ms  {name}')


if __name__ == '__main__':
    main(sys.argv[1])
