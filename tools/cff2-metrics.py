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

The output starts with comment lines naming the HarfBuzz version, the
variations and the extents ("# extents: ASCENDER DESCENDER LINE_GAP"),
then a glyph id and its advance per line.
"""
import ctypes
import ctypes.util
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
extents = Extents()
hb.hb_font_get_h_extents(font, ctypes.byref(extents))
lines = [
    f'# HarfBuzz {hb.hb_version_string().decode()}, from tools/cff2-metrics.py',
    *([f'# variations: {variations}'] if variations else []),
    f'# extents: {extents.ascender} {extents.descender} {extents.line_gap}',
]
lines += [f'{gid} {hb.hb_font_get_glyph_h_advance(font, gid)}' for gid in range(hb.hb_face_get_glyph_count(face))]
with open(out_path, 'w') as out:
    out.write('\n'.join(lines) + '\n')
