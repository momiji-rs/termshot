#!/usr/bin/env python3
"""Record the metrics HarfBuzz gives a variable font at an instance, which
src/metrics_tests.rs checks termshot's HVAR and MVAR against: the
horizontal advance of every glyph, and the ascender, descender and line gap.

    tools/cff2-metrics.py [--variations=LIST] FONT OUT

--variations takes hb-view's list (wght=700,wdth=100); without it the
default instance is recorded. It calls libharfbuzz through ctypes, so it
needs only the library, and asks for each glyph by id, as no shaping is
involved. The font is read at its units per em, so every value is in font
units, as HarfBuzz rounds it.

The extents are hhea's, as termshot reads them (stb_truetype does), varied
by HarfBuzz's MVAR deltas (hb_ot_metrics_get_variation) and rounded as
hb_font_get_h_extents rounds them. hb_font_get_h_extents itself starts from
OS/2's typo metrics instead when fsSelection sets USE_TYPO_METRICS, so it
is only a cross-check: where it starts from hhea, the two must agree.

The output starts with comment lines naming the HarfBuzz version, the
variations and the extents ("# extents: ASCENDER DESCENDER LINE_GAP"),
then a glyph id and its advance per line.
"""
import ctypes
import ctypes.util
import math
import struct
import sys

args = sys.argv[1:]
variations = ''
if args and args[0].startswith('--variations='):
    variations = args.pop(0)[len('--variations='):]
if len(args) != 2:
    sys.exit(__doc__.strip().split('\n\n')[1])
font_path, out_path = args

hb = ctypes.CDLL(ctypes.util.find_library('harfbuzz') or 'libharfbuzz.so.0')
vp = ctypes.c_void_p
hb.hb_version_string.restype = ctypes.c_char_p
hb.hb_blob_create_from_file_or_fail.restype = vp
hb.hb_blob_create_from_file_or_fail.argtypes = [ctypes.c_char_p]
hb.hb_face_create.restype = vp
hb.hb_face_create.argtypes = [vp, ctypes.c_uint]
hb.hb_face_get_glyph_count.argtypes = [vp]
hb.hb_font_create.restype = vp
hb.hb_font_create.argtypes = [vp]
hb.hb_font_set_variations.argtypes = [vp, vp, ctypes.c_uint]
hb.hb_font_get_glyph_h_advance.restype = ctypes.c_int32
hb.hb_font_get_glyph_h_advance.argtypes = [vp, ctypes.c_uint32]
hb.hb_font_get_h_extents.argtypes = [vp, vp]
hb.hb_face_reference_table.restype = vp
hb.hb_face_reference_table.argtypes = [vp, ctypes.c_uint32]
hb.hb_blob_get_data.restype = ctypes.POINTER(ctypes.c_char)
hb.hb_blob_get_data.argtypes = [vp, ctypes.POINTER(ctypes.c_uint)]
hb.hb_ot_metrics_get_variation.restype = ctypes.c_float
hb.hb_ot_metrics_get_variation.argtypes = [vp, ctypes.c_uint32]


class Variation(ctypes.Structure):
    _fields_ = [('tag', ctypes.c_uint32), ('value', ctypes.c_float)]


class Extents(ctypes.Structure):
    _fields_ = [('ascender', ctypes.c_int32), ('descender', ctypes.c_int32),
                ('line_gap', ctypes.c_int32)] + [(f'reserved{i}', ctypes.c_int32) for i in range(9)]


hb.hb_variation_from_string.argtypes = [ctypes.c_char_p, ctypes.c_int, ctypes.POINTER(Variation)]

blob = hb.hb_blob_create_from_file_or_fail(font_path.encode())
if not blob:
    sys.exit(f'{font_path}: cannot read it')
face = hb.hb_face_create(blob, 0)
font = hb.hb_font_create(face)
if variations:
    settings = (Variation * len(variations.split(',')))()
    for i, setting in enumerate(variations.split(',')):
        if not hb.hb_variation_from_string(setting.encode(), -1, ctypes.byref(settings[i])):
            sys.exit(f'{setting!r} is not a variation setting')
    hb.hb_font_set_variations(font, settings, len(settings))


def tag(name):
    return struct.unpack('>I', name)[0]


def table(name):
    length = ctypes.c_uint()
    data = hb.hb_blob_get_data(hb.hb_face_reference_table(face, tag(name)), ctypes.byref(length))
    return ctypes.string_at(data, length.value) if length.value else b''


def f32(v):
    return struct.unpack('f', struct.pack('f', v))[0]


def roundf(v):
    # HarfBuzz's roundf, floorf(v + 0.5f), in f32.
    return math.floor(f32(v + 0.5))


def varied(name, base):
    return f32(base + hb.hb_ot_metrics_get_variation(font, tag(name)))


hhea = table(b'hhea')
if len(hhea) < 10:
    sys.exit(f'{font_path}: no hhea table')
ascender, descender, line_gap = struct.unpack('>hhh', hhea[4:10])
ascender = roundf(abs(varied(b'hasc', ascender)))
descender = roundf(-abs(varied(b'hdsc', descender)))
line_gap = roundf(varied(b'hlgp', line_gap))
os2 = table(b'OS/2')
if not (len(os2) >= 64 and struct.unpack('>H', os2[62:64])[0] & 0x80):
    extents = Extents()
    hb.hb_font_get_h_extents(font, ctypes.byref(extents))
    harfbuzz = (extents.ascender, extents.descender, extents.line_gap)
    if harfbuzz != (ascender, descender, line_gap):
        sys.exit(f'{font_path}: hb_font_get_h_extents gives {harfbuzz}, hhea and MVAR {(ascender, descender, line_gap)}')
lines = [
    f'# HarfBuzz {hb.hb_version_string().decode()}, from tools/cff2-metrics.py',
    *([f'# variations: {variations}'] if variations else []),
    f'# extents: {ascender} {descender} {line_gap}',
]
lines += [f'{gid} {hb.hb_font_get_glyph_h_advance(font, gid)}' for gid in range(hb.hb_face_get_glyph_count(face))]
with open(out_path, 'w') as out:
    out.write('\n'.join(lines) + '\n')
