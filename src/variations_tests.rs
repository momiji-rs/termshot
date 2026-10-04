//! Tests for src/variations.rs: axis settings, fvar and avar, against
//! HarfBuzz's normalization.

use crate::cff_tests::craft;
use crate::font;
use crate::variations::{self, Axis};
use std::fs;
use std::path::Path;

/// F2Dot14 units.
fn f2dot14(v: f64) -> i16 {
    (v * 16384.0).round() as i16
}

/// Every case of HarfBuzz's normalization and of its avar segment maps, an
/// axis each: (tag, minimum, default, maximum, segment map, setting).
fn cases() -> Vec<(&'static [u8; 4], f64, f64, f64, Vec<(f64, f64)>, &'static str)> {
    let identity = || vec![(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)];
    vec![
        (b"lin ", 100.0, 400.0, 900.0, identity(), "650"),
        (b"low ", 100.0, 400.0, 900.0, identity(), "250"),
        (b"clmp", 100.0, 400.0, 900.0, identity(), "1000"),
        (b"clmn", 100.0, 400.0, 900.0, identity(), "-5"),
        (b"frac", 100.0, 400.0, 900.0, identity(), "133.3"),
        (b"deft", 100.0, 400.0, 900.0, identity(), "400"),
        // A minimum over the default and a maximum under it: only the default.
        (b"odd ", 500.0, 400.0, 300.0, identity(), "450"),
        // Rounded to 2.14 with (c + 2) >> 2, which floors.
        (b"tiny", -1.0, 0.0, 1.0, identity(), "-0.0000457"),
        (b"none", -1.0, 0.0, 1.0, vec![], "0.3"),
        (b"one ", -1.0, 0.0, 1.0, vec![(0.5, 0.25)], "0.75"),
        (b"lerp", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.0), (0.5, 0.8), (1.0, 1.0)], "0.25"),
        (b"lrpn", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (-0.5, -0.2), (0.0, 0.0), (1.0, 1.0)], "-0.75"),
        // A leading (-1, -1) before another -1 is dropped, so two match, not three.
        (b"skp-", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (-1.0, -0.5), (-1.0, -0.2), (0.0, 0.0), (1.0, 1.0)], "-1"),
        // A trailing (1, 1) after another 1 likewise.
        (b"skp+", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.0), (1.0, 0.3), (1.0, 0.6), (1.0, 1.0)], "1"),
        // Three matches give the middle one.
        (b"zer3", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, -0.3), (0.0, 0.1), (0.0, 0.5), (1.0, 1.0)], "0"),
        // Two at 0 give the one nearer 0, the last on a tie.
        (b"zer2", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.2), (0.0, -0.1), (1.0, 1.0)], "0"),
        (b"tie0", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.2), (0.0, -0.2), (1.0, 1.0)], "0"),
        // Above 0 the first of them, below 0 the last, whichever is nearer 0.
        (b"pos4", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.0), (0.5, 0.4), (0.5, 0.3), (0.5, 0.2), (0.5, 0.1), (1.0, 1.0)], "0.5"),
        (b"neg2", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (-0.5, -0.6), (-0.5, -0.7), (0.0, 0.0), (1.0, 1.0)], "-0.5"),
        // Before every segment or after them, shifted.
        (b"befr", -1.0, 0.0, 1.0, vec![(-0.5, -0.25), (1.0, 1.0)], "-1"),
        (b"aftr", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.5, 0.25)], "1"),
        // Unsorted: the segment ends at the first `from` past the value.
        (b"unst", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.5, 0.5), (0.0, 0.0), (1.0, 1.0)], "0.25"),
        // An axis not set is at its default, which avar maps too.
        (b"rest", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.5), (1.0, 1.0)], ""),
        // A lone (-1, -1), with nothing after it to skip to.
        (b"sole", -1.0, 0.0, 1.0, vec![(-1.0, -1.0)], "0.5"),
        // A trailing (1, 1) after a lower `from` is a segment's end.
        (b"last", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.0), (0.5, 0.25), (1.0, 1.0)], "0.75"),
        // f32 details, each 1/16384 off if missed: interpolation multiplies
        // before it divides, and the mapped value is rounded, not cut.
        (b"ordr", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.0), (14833.0 / 16384.0, 13829.0 / 16384.0), (1.0, 1.0)], "0.8829"),
        (b"rond", -1.0, 0.0, 1.0, vec![(-1.0, -1.0), (0.0, 0.0), (151.0 / 16384.0, 15388.0 / 16384.0), (1.0, 1.0)], "0.006"),
    ]
}

/// The coordinate hb-vector 14.4.0 draws for each of cases(), on starship:
/// glyph k + 1 of `hb-vector --font-size=1000 --precision=9
/// --variations=SETTINGS --glyphs target/test/variations.otf gid<k+1>`
/// moves to x = the coordinate, as cff2_coordinates draws it, with SETTINGS
/// from target/test/variations.txt. hb_font_get_var_coords_normalized gives
/// the same.
const HARFBUZZ: [i32; 27] = [
    8192, -8192, 16384, -16384, -14565, 0, 0, -1, 4915, 8192, 6554, -9830, -3277, 4915, 1638, -1638, -3277, 6554, -11469,
    -12288, 12288, 4096, 8192, 8192, 10240, 13486, 10013,
];

/// The test font of cases(), and its settings.
fn font_of_cases() -> (Vec<u8>, String) {
    let cases = cases();
    let axes: Vec<_> = cases.iter().map(|&(tag, min, default, max, ..)| (tag, min, default, max)).collect();
    let maps: Vec<Vec<(i16, i16)>> =
        cases.iter().map(|(.., map, _)| map.iter().map(|&(from, to)| (f2dot14(from), f2dot14(to))).collect()).collect();
    let (fvar, avar) = (craft::fvar(&axes), craft::avar(&maps));
    let settings: Vec<String> = cases
        .iter()
        .filter(|(.., setting)| !setting.is_empty())
        .map(|(tag, .., setting)| format!("{}={setting}", variations::tag_name(tag)))
        .collect();
    (craft::cff2_coordinates(&fvar, Some(&avar), cases.len() as u16), settings.join(","))
}

#[test]
fn axis_settings_normalize_as_in_harfbuzz() {
    let (data, settings) = font_of_cases();
    fs::create_dir_all("target/test").unwrap();
    fs::write("target/test/variations.otf", &data).unwrap();
    fs::write("target/test/variations.txt", format!("{settings}\n")).unwrap();
    let spec = font::Spec { path: "target/test/variations.otf".into(), face: None, axes: Some(settings) };
    let font = font::load(&spec).unwrap();
    assert_eq!(font.coords, HARFBUZZ);
    let cff = font::cff_outlines(&font.data, font.start, &font.coords).unwrap().unwrap();
    let (mut out, mut bounds) = (Vec::new(), [0; 4]);
    for (k, &x) in HARFBUZZ.iter().enumerate() {
        assert_eq!(cff.glyph(k + 1, &mut out, &mut bounds), Ok(true));
        assert_eq!((out[0].x as i32, out[0].y), (x, 0), "glyph {}", k + 1);
    }
}

/// Loads `data` as target/test/`name` with axis settings `axes`.
fn load_with(name: &str, data: &[u8], axes: &str) -> Result<font::Font, String> {
    let path = format!("target/test/{name}");
    fs::create_dir_all("target/test").unwrap();
    fs::write(&path, data).unwrap();
    font::load(&font::Spec { path, face: None, axes: Some(axes.into()) })
}

fn weight() -> Axis {
    Axis { tag: *b"wght", min: 100.0, default: 400.0, max: 900.0 }
}

fn width() -> Axis {
    Axis { tag: *b"wdth", min: 50.0, default: 100.0, max: 100.0 }
}

#[test]
fn axis_settings_parse_strictly() {
    for (settings, parsed) in [
        ("wght=700", vec![(*b"wght", 700.0)]),
        ("wght=700,wdth=87.5", vec![(*b"wght", 700.0), (*b"wdth", 87.5)]),
        ("ab=-1e2", vec![(*b"ab  ", -100.0)]),
        ("ital=1,ital=0", vec![(*b"ital", 1.0), (*b"ital", 0.0)]),
        ("w_t!=.5", vec![(*b"w_t!", 0.5)]),
    ] {
        assert_eq!(variations::parse(settings), Ok(parsed), "{settings}");
    }
    for (settings, error) in [
        ("", r#""" is not an axis setting; give them as TAG=VALUE"#),
        ("wght", r#""wght" is not an axis setting"#),
        ("wght=700,", r#""" is not an axis setting"#),
        ("=700", r#""" is not an axis tag"#),
        ("weight=700", r#""weight" is not an axis tag: tags are 1 to 4 printable ASCII characters other than a space"#),
        ("weigh=700", r#""weigh" is not an axis tag"#),
        ("w t=1", r#""w t" is not an axis tag"#),
        ("wgh\u{e9}=1", "is not an axis tag"),
        ("wght=", r#""wght=": the value of an axis is a number, such as wght=700"#),
        ("wght=bold", r#""wght=bold": the value"#),
        ("wght=inf", r#""wght=inf": the value"#),
        ("wght=NaN", r#""wght=NaN": the value"#),
        ("wght=1e999", r#""wght=1e999": the value"#),
    ] {
        let got = variations::parse(settings).unwrap_err();
        assert!(got.contains(error), "{settings}: {got}");
    }
}

#[test]
fn fvar_is_checked_as_harfbuzz_checks_it() {
    let good = craft::fvar(&[(b"wght", 100.0, 400.0, 900.0), (b"wdth", 50.0, 100.0, 100.0)]);
    assert_eq!(variations::axes(&good), Ok(vec![weight(), width()]));
    let set = |at: usize, v: u16| {
        let mut fvar = good.clone();
        fvar[at..at + 2].copy_from_slice(&v.to_be_bytes());
        fvar
    };
    for (fvar, error) in [
        (set(0, 0), "version 0"),
        (set(0, 2), "version 2"),
        (set(10, 24), "axis records of 24 bytes, not 20"),
        (set(14, 11), "instance records of 11 bytes, for 2 axes"),
        // One instance record, which isn't in the table.
        (set(12, 1), "its axes and instances run past the table"),
        (set(4, 20), "its axes and instances run past the table"),
        (good[..good.len() - 1].to_vec(), "its axes and instances run past the table"),
        (good[..15].to_vec(), "truncated at byte 14"),
    ] {
        assert_eq!(variations::axes(&fvar), Err(error.into()));
    }
    // The axes start where fvar says.
    let mut later = set(4, 20);
    later.splice(16..16, [0; 4]);
    assert_eq!(variations::axes(&later), Ok(vec![weight(), width()]));
}

#[test]
fn avar_is_version_1_with_a_map_per_axis() {
    let avar = craft::avar(&[vec![(-16384, -16384), (0, 0), (8192, 4096), (16384, 16384)], vec![]]);
    let maps = vec![vec![(-1.0, -1.0), (0.0, 0.0), (0.5, 0.25), (1.0, 1.0)], vec![]];
    assert_eq!(variations::segment_maps(&avar, 2), Ok(maps));
    assert_eq!(variations::segment_maps(&avar, 3), Err("it maps 2 axes, and fvar has 3".into()));
    assert_eq!(variations::segment_maps(&avar, 1), Err("it maps 2 axes, and fvar has 1".into()));
    let mut v2 = avar.clone();
    v2[1] = 2;
    assert_eq!(variations::segment_maps(&v2, 2), Err("version 2; only version 1 is supported".into()));
    assert_eq!(variations::segment_maps(&avar[..avar.len() - 2], 2), Err(format!("truncated at byte {}", avar.len() - 2)));
    assert_eq!(variations::segment_maps(&avar[..9], 2), Err("truncated at byte 8".into()));
}

#[test]
fn messages_name_the_axes_and_the_instance() {
    // A minimum over the default is clamped to it, as in HarfBuzz.
    let odd = Axis { tag: *b"ab  ", min: 2.0, default: 1.0, max: 3.5 };
    assert_eq!(variations::describe(&[]), "it has no axes");
    assert_eq!(variations::describe(&[weight()]), "its axis is wght 100 to 900, default 400");
    assert_eq!(
        variations::describe(&[weight(), width(), odd]),
        "its axes are wght 100 to 900, default 400; wdth 50 to 100, default 100; ab 1 to 3.5, default 1"
    );
    let axes = [weight(), width()];
    let instance = |settings| variations::instance(&axes, &variations::parse(settings).unwrap());
    assert_eq!(instance("wdth=120,wght=350.5,wght=1000"), "wght=900 (1000 clamped),wdth=100 (120 clamped)");
    assert_eq!(instance("wght=700"), "wght=700");
    let error = variations::coords(&axes, &[], &variations::parse("wght=700,opsz=12").unwrap()).unwrap_err();
    assert_eq!(error, r#"no axis "opsz"; its axes are wght 100 to 900, default 400; wdth 50 to 100, default 100"#);
}

#[test]
fn a_last_part_with_an_equals_sign_is_axis_settings() {
    for (value, face, axes) in [
        ("target/test/absent.otf#wght=700", None, Some("wght=700")),
        ("target/test/absent.ttc#1#wght=700,wdth=50", Some("1"), Some("wght=700,wdth=50")),
        ("target/test/absent.ttc#Foo #1#wght=700", Some("Foo #1"), Some("wght=700")),
        ("target/test/absent.ttc#1", Some("1"), None),
        ("target/test/absent.ttc#wght=700#1", Some("wght=700#1"), None),
    ] {
        let spec = font::Spec::parse(value).unwrap();
        let path = value[..value.find('#').unwrap()].into();
        assert_eq!(spec, font::Spec { path, face: face.map(Into::into), axes: axes.map(Into::into) }, "{value}");
        assert_eq!(spec.name(), value);
    }
    let error = font::Spec::parse("target/test/absent.otf#wght=bold").unwrap_err();
    assert_eq!(error, r#"target/test/absent.otf#wght=bold: "wght=bold": the value of an axis is a number, such as wght=700"#);
    assert!(!Path::new("target/test/absent.otf").exists());
}

#[test]
fn only_cff2_outlines_with_the_axis_vary() {
    let jetbrains = fs::read("third_party/jetbrains-mono/JetBrainsMono-Regular.ttf").unwrap();
    let (data, _) = font_of_cases();
    let with_version_2 = |header: &[u8]| {
        let mut data = data.clone();
        let at = data.windows(header.len()).position(|w| w == header).unwrap();
        data[at + 1] = 2;
        data
    };
    // fvar's and avar's headers, as craft writes them.
    let axes = cases().len() as u8;
    let fvar_2 = with_version_2(&[0, 1, 0, 0, 0, 16, 0, 2, 0, axes]);
    let avar_2 = with_version_2(&[0, 1, 0, 0, 0, 0, 0, axes]);
    let only = "cannot set wght=700: termshot varies CFF2 outlines only, and this face has";
    for (name, data, axes, error) in [
        ("vary.ttf", jetbrains, "wght=700", format!("{only} TrueType (glyf) ones")),
        ("vary-cff.otf", craft::control(), "wght=700", format!("{only} CFF ones, which do not vary")),
        ("vary-cff2.otf", craft::cff2_control(), "wght=700", "cannot set wght=700: the face has no axes (no fvar table)".into()),
        ("vary-axis.otf", data, "wdth=50", r#"no axis "wdth"; its axes are lin 100 to 900, default 400; low "#.into()),
        ("vary-fvar.otf", fvar_2, "lin=500", "not a usable font: fvar: version 2".into()),
        ("vary-avar.otf", avar_2, "lin=500", "not a usable font: avar: version 2; only version 1 is supported".into()),
    ] {
        let got = load_with(name, &data, axes).err().unwrap_or_else(|| panic!("{name} loaded"));
        assert!(got.starts_with(&format!("target/test/{name}: {error}")), "{name}: {got}");
    }
}

/// HarfBuzz draws coordinates that are all 0 as the default instance,
/// without blending, though a region that peaks at 0 counts 1 anywhere
/// else on the axis: hb-vector moves glyph 1 of target/test/peak-0.otf to
/// x = 0 with no --variations or with ax0=0, and to 16384 with ax0=0.5.
#[test]
fn coordinates_of_0_are_the_default_instance() {
    let data = craft::cff2_scalars(1, &[vec![[0, 0, 0x4000]]]);
    let first_x = |font: &font::Font| {
        let cff = font::cff_outlines(&font.data, font.start, &font.coords).unwrap().unwrap();
        let (mut out, mut bounds) = (Vec::new(), [0; 4]);
        assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true));
        out[0].x
    };
    let font = font::prepare(data.clone()).unwrap();
    assert_eq!((first_x(&font), font.coords, font.instance), (0, vec![], None));
    // A setting at the default is the same instance, and -v names it.
    let font = load_with("peak-0.otf", &data, "ax0=0").unwrap();
    assert_eq!((first_x(&font), &font.coords[..], font.instance.as_deref()), (0, &[0][..], Some("ax0=0")));
    let font = load_with("peak-0.otf", &data, "ax0=0.5").unwrap();
    assert_eq!((first_x(&font), &font.coords[..]), (16384, &[8192][..]));
}

/// A setting sets every axis with its tag, as hb_font_set_variations does:
/// with two axes `ax0 `, `ax0=0.5` puts both of the region's axes at 0.5,
/// and hb-vector moves to 16384 × 0.5 × 0.5 = 4096 (14.4.0), not to 0 as
/// for the first axis alone.
#[test]
fn a_setting_sets_every_axis_with_its_tag() {
    let up = [0, 0x4000, 0x4000];
    let fvar = craft::fvar(&[(b"ax0 ", -1.0, 0.0, 1.0), (b"ax0 ", -1.0, 0.0, 1.0)]);
    let font = load_with("two-ax0.otf", &craft::cff2_scalars_with(&fvar, 2, &[vec![up, up]]), "ax0=0.5").unwrap();
    let cff = font::cff_outlines(&font.data, font.start, &font.coords).unwrap().unwrap();
    let (mut out, mut bounds) = (Vec::new(), [0; 4]);
    assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true));
    assert_eq!((&font.coords[..], out[0].x), (&[8192, 8192][..], 4096));
}

/// HarfBuzz reads a region list axis that fvar lacks as 0, and ignores an
/// fvar axis past the list's; termshot refuses either at an instance, and
/// draws the default instance, which reads no axes, as before.
#[test]
fn the_store_has_the_axes_of_fvar_at_an_instance() {
    let up = [0, 0x4000, 0x4000];
    let one = craft::fvar(&[(b"ax0 ", -1.0, 0.0, 1.0)]);
    let two = craft::fvar(&[(b"ax0 ", -1.0, 0.0, 1.0), (b"ax1 ", -1.0, 0.0, 1.0)]);
    for (fvar, axes, region, store, fvar_axes) in [(&one, 2, vec![up, up], 2, 1), (&two, 1, vec![up], 1, 2)] {
        let data = craft::cff2_scalars_with(fvar, axes, &[region]);
        assert!(font::prepare(data.clone()).is_ok());
        let err = load_with("axes-differ.otf", &data, "ax0=0.5").err().unwrap();
        let reason = format!("the region list's axis count is {store} and fvar's {fvar_axes}");
        assert!(err.contains(&reason), "{err}");
        // A setting at the default is still checked.
        assert!(load_with("axes-differ.otf", &data, "ax0=0").is_err());
    }
}
