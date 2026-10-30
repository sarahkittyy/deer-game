# Makes every animation in a GLB key every bone that any animation keys, filling the missing
# channels with the bone's rest pose (constant). Bevy normalises blends per bone across only
# the clips that key it, so a bone missing from the main clip otherwise takes a faint clip's
# pose at full strength.
import json, struct, sys

def load(path):
    b = open(path, 'rb').read()
    jl = struct.unpack('<I', b[12:16])[0]
    j = json.loads(b[20:20 + jl])
    off = 20 + jl
    bl = struct.unpack('<I', b[off:off + 4])[0]
    return j, bytearray(b[off + 8: off + 8 + bl])

def save(path, j, binc):
    while len(binc) % 4: binc.append(0)
    j['buffers'][0]['byteLength'] = len(binc)
    js = json.dumps(j, separators=(',', ':')).encode()
    while len(js) % 4: js += b' '
    out = bytearray()
    total = 12 + 8 + len(js) + 8 + len(binc)
    out += struct.pack('<4sII', b'glTF', 2, total)
    out += struct.pack('<I4s', len(js), b'JSON') + js
    out += struct.pack('<I4s', len(binc), b'BIN\0') + binc
    open(path, 'wb').write(out)

def add_accessor(j, binc, floats, count, typ, minmax=False):
    while len(binc) % 4: binc.append(0)
    off = len(binc)
    binc += struct.pack('<%df' % len(floats), *floats)
    j['bufferViews'].append({'buffer': 0, 'byteOffset': off, 'byteLength': 4 * len(floats)})
    acc = {'bufferView': len(j['bufferViews']) - 1, 'componentType': 5126, 'count': count, 'type': typ}
    if minmax:
        acc['min'] = [min(floats)]; acc['max'] = [max(floats)]
    j['accessors'].append(acc)
    return len(j['accessors']) - 1

def main(path):
    j, binc = load(path)
    nodes = j['nodes']
    keyed = set()
    for a in j['animations']:
        for ch in a['channels']:
            keyed.add((ch['target']['node'], ch['target']['path']))
    rest = {'translation': lambda n: n.get('translation', [0, 0, 0]),
            'rotation': lambda n: n.get('rotation', [0, 0, 0, 1]),
            'scale': lambda n: n.get('scale', [1, 1, 1])}
    added = 0
    for a in j['animations']:
        have = {(ch['target']['node'], ch['target']['path']) for ch in a['channels']}
        end = 0.0
        for s in a['samplers']:
            end = max(end, j['accessors'][s['input']]['max'][0])
        times = add_accessor(j, binc, [0.0, max(end, 1e-3)], 2, 'SCALAR', minmax=True)
        for (node, prop) in sorted(keyed - have):
            v = rest[prop](nodes[node])
            out = add_accessor(j, binc, list(v) * 2, 2, "VEC4" if prop == "rotation" else "VEC3")
            a['samplers'].append({'input': times, 'output': out, 'interpolation': 'LINEAR'})
            a["channels"].append({"sampler": len(a["samplers"]) - 1, "target": {"node": node, "path": prop}})
            added += 1
    save(path, j, binc)
    print(path, 'added', added, 'rest channels')

for p in sys.argv[1:]:
    main(p)
