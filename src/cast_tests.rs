//! asciicast input: the JSON reader, v2 and v3 headers and events, resizes,
//! and every way a cast can be malformed.

use super::*;
use cast::{decode, detect, parse, JsonError, Value, MAX_DEPTH};

fn decode_str(text: &str) -> Result<cast::Cast, String> {
    decode(text.as_bytes())
}

fn error(text: &str) -> String {
    decode_str(text).unwrap_err()
}

fn string(json: &str) -> String {
    match parse(json) {
        Ok(Value::String(s)) => s,
        other => panic!("{json}: {other:?}"),
    }
}

#[test]
fn json_values() {
    assert_eq!(parse(" null "), Ok(Value::Null));
    assert_eq!(parse("true"), Ok(Value::Bool(true)));
    assert_eq!(parse("false"), Ok(Value::Bool(false)));
    assert_eq!(parse("-0.5e+10"), Ok(Value::Number("-0.5e+10")));
    assert_eq!(parse("[1, [], {}]"), Ok(Value::Array(vec![Value::Number("1"), Value::Array(vec![]), Value::Object(vec![])])));
    assert_eq!(
        parse(r#"{"a": "b", "c": [null]}"#),
        Ok(Value::Object(vec![("a".into(), Value::String("b".into())), ("c".into(), Value::Array(vec![Value::Null]))]))
    );
}

#[test]
fn json_escapes() {
    assert_eq!(string(r#""\" \\ \/ \b \f \n \r \t""#), "\" \\ / \u{8} \u{c} \n \r \t");
    assert_eq!(string(r#""\u001b[1m\u00e9\u4e2D""#), "\x1b[1mé中");
    // A surrogate pair is one character, in UTF-8 four bytes.
    assert_eq!(string(r#""\ud83d\ude00!""#), "😀!");
    assert_eq!(string(r#""\uD83D\uDE00""#).as_bytes(), [0xf0, 0x9f, 0x98, 0x80]);
    // Raw UTF-8 passes through unchanged.
    assert_eq!(string("\"é中😀\""), "é中😀");
    assert_eq!(string(r#""\u0000""#), "\0");
}

#[test]
fn json_rejects_what_rfc_8259_does() {
    for bad in [
        "", "[", "[1,]", "[1 2]", "{\"a\"}", "{\"a\":1,}", "{a:1}", "'a'", "01", "-", "1.", ".5", "1e", "+1",
        "NaN", "Infinity", "tru", "nul", "\"abc", "\"\\x\"", "\"\\u12\"", "\"\\u12g4\"", "\"a\tb\"", "\"\u{1}\"",
        "1 2", "[] x", "{\"a\":1,\"a\":2}",
        // Lone and misordered surrogates have no UTF-8 form.
        r#""\ud800""#, r#""\udc00""#, r#""\ud800x""#, r#""\ud800\u0041""#, r#""\ude00\ud83d""#, r#""\ud800\ud800""#,
    ] {
        assert!(parse(bad).is_err(), "{bad:?} accepted: {:?}", parse(bad));
    }
    assert_eq!(parse("[1,]").unwrap_err().at, 3);
    let JsonError { reason, .. } = parse(r#"{"k": 1, "k": 2}"#).unwrap_err();
    assert!(reason.contains("appears twice"), "{reason}");
}

#[test]
fn json_nesting_is_bounded() {
    let nested = |n: usize| "[".repeat(n) + &"]".repeat(n);
    assert!(parse(&nested(MAX_DEPTH)).is_ok());
    let deep = parse(&nested(MAX_DEPTH + 1)).unwrap_err();
    assert!(deep.reason.contains("nest more than"), "{}", deep.reason);
    // Far deeper than any stack could recurse: refused at the limit, not overflowed.
    assert!(parse(&nested(1_000_000)).is_err());
    let objects = "{\"a\":".repeat(MAX_DEPTH + 1) + "1" + &"}".repeat(MAX_DEPTH + 1);
    assert!(parse(&objects).is_err());
}

const V2: &str = "{\"version\": 2, \"width\": 20, \"height\": 4, \"timestamp\": 1700000000, \"env\": {\"TERM\": \"xterm-256color\"}}\n\
[0.1, \"o\", \"\\u001b[3\"]\n\
[0.2, \"i\", \"x\"]\n\
[0.25, \"o\", \"1mred\\u001b[m\\r\\n\"]\n\
[0.3, \"m\", \"\"]\n\
[0.4, \"o\", \"\\u00e9\\ud83d\\ude00\"]\n";

#[test]
fn v2_replays_output_events_in_order() {
    let cast = decode_str(V2).unwrap();
    assert_eq!(cast.version, 2);
    assert_eq!((cast.size, cast.final_size, cast.resized), ((20, 4), (20, 4), false));
    assert_eq!(cast.output, "\x1b[31mred\x1b[m\r\né😀".as_bytes());
    assert!(detect(V2.as_bytes()));
}

#[test]
fn v3_has_term_intervals_and_comments() {
    let v3 = "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24,\"type\":\"xterm-256color\",\
\"theme\":{\"fg\":\"#ffffff\",\"bg\":\"#000000\",\"palette\":\"#000000:#111111\"}},\"timestamp\":1}\n\
# a comment, ignored\n\
[0.5, \"o\", \"a\"]\n\
\n\
[0.0, \"x\", \"0\"]\n\
[1.25, \"o\", \"b\"]\n";
    let cast = decode_str(v3).unwrap();
    assert_eq!((cast.version, cast.size, cast.output.as_slice()), (3, (80, 24), &b"ab"[..]));
    // A comment is v3's; in v2 it is a malformed event.
    let v2 = "{\"version\":2,\"width\":1,\"height\":1}\n# no\n";
    assert!(error(v2).starts_with("line 2, column 1:"), "{}", error(v2));
}

#[test]
fn resize_events_give_the_final_size() {
    let cast = decode_str("{\"version\":2,\"width\":80,\"height\":24}\n[0,\"o\",\"a\"]\n[1,\"r\",\"100x30\"]\n[2,\"r\",\"120x40\"]\n[3,\"o\",\"b\"]\n").unwrap();
    assert_eq!((cast.size, cast.final_size, cast.resized), ((80, 24), (120, 40), true));
    assert_eq!(cast.output, b"ab");
    for bad in ["\"100\"", "\"x30\"", "\"100x\"", "\"100X30\"", "\"-1x30\"", "\"1 x30\"", "100", "\"99999999999999999999x1\""] {
        let text = format!("{{\"version\":3,\"term\":{{\"cols\":1,\"rows\":1}}}}\n[0,\"r\",{bad}]\n");
        assert!(error(&text).starts_with("line 2: "), "{bad}: {}", error(&text));
    }
}

#[test]
fn crlf_lines_and_a_missing_final_newline_are_fine() {
    let cast = decode_str("{\"version\":2,\"width\":3,\"height\":2}\r\n[0,\"o\",\"x\"]\r\n[1,\"o\",\"y\"]").unwrap();
    assert_eq!(cast.output, b"xy");
    assert_eq!(decode_str("{\"version\":2,\"width\":3,\"height\":2}").unwrap().output, b"");
}

#[test]
fn detection_needs_a_version_in_a_first_line_object() {
    for cast in ["{\"version\":2}", "{\"width\":1,\"version\":3}\n", "{\"version\":7}\nanything"] {
        assert!(detect(cast.as_bytes()), "{cast}");
    }
    for raw in [
        "", "\x1b[1mhi", "hello\n{\"version\":2}", " {\"version\":2}", "{\"Version\":2}", "{\"a\":{\"version\":2}}",
        "{\"version\":2", "{\"version\":2} trailing", "[\"version\",2]", "{}",
    ] {
        assert!(!detect(raw.as_bytes()), "{raw}");
    }
    assert!(!detect(b"{\"version\":2,\"x\":\"\xff\"}"));
}

#[test]
fn headers_are_checked() {
    let cases = [
        ("{\"version\":1,\"width\":1,\"height\":1}", "version 1 is not supported"),
        ("{\"version\":4}", "version 4 is not supported"),
        ("{\"version\":\"2\",\"width\":1,\"height\":1}", "the version must be a number, not a string"),
        ("{\"width\":1,\"height\":1}", "no version"),
        ("[2]", "must be a JSON object, not an array"),
        ("{\"version\":2,\"height\":1}", "no width"),
        ("{\"version\":2,\"width\":1}", "no height"),
        ("{\"version\":2,\"width\":\"80\",\"height\":1}", "width must be a whole number, not a string"),
        ("{\"version\":2,\"width\":80.0,\"height\":1}", "width must be a whole number, not 80.0"),
        ("{\"version\":2,\"width\":-1,\"height\":1}", "width must be a whole number, not -1"),
        ("{\"version\":2,\"width\":1e3,\"height\":1}", "width must be a whole number, not 1e3"),
        ("{\"version\":2,\"width\":18446744073709551616,\"height\":1}", "width 18446744073709551616 is too large"),
        ("{\"version\":3,\"width\":1,\"height\":1}", "no term"),
        ("{\"version\":3,\"term\":[80,24]}", "term must be an object"),
        ("{\"version\":3,\"term\":{\"cols\":80}}", "no term.rows"),
        ("{\"version\":3,\"term\":{\"rows\":24}}", "no term.cols"),
        ("not json", "column 1: expected a JSON value"),
        ("", "the line ends where a JSON value was expected"),
    ];
    for (text, want) in cases {
        let got = error(text);
        assert!(got.starts_with("line 1") && got.contains(want), "{text}: {got}");
    }
    // The largest 64-bit size reads; whether it can be drawn is the CLI's call.
    let max = decode_str("{\"version\":2,\"width\":18446744073709551615,\"height\":0}").unwrap();
    assert_eq!(max.size, (u64::MAX, 0));
}

#[test]
fn events_are_checked() {
    let header = "{\"version\":2,\"width\":10,\"height\":2}\n[0,\"o\",\"ok\"]\n";
    let cases = [
        ("{\"o\":1}", "line 3: an event must be an array"),
        ("[0,\"o\"]", "line 3: an event must be [time, code, data], not 2 items"),
        ("[0,\"o\",\"a\",1]", "not 4 items"),
        ("[\"0\",\"o\",\"a\"]", "time must be a finite number"),
        ("[-1,\"o\",\"a\"]", "time must be a finite number"),
        ("[1e999,\"o\",\"a\"]", "time must be a finite number"),
        ("[0,1,\"a\"]", "code must be a string, not a number"),
        ("[0,\"o\",1]", "data must be a string, not a number"),
        ("[0,\"o\",null]", "data must be a string, not null"),
        ("[0,\"o\",\"a\"", "line 3, column 11: the line ends where ',' or ']' was expected"),
        ("[0,\"o\",\"\\ud800\"]", "line 3, column 9: a lone UTF-16 surrogate"),
    ];
    for (event, want) in cases {
        let got = error(&format!("{header}{event}\n"));
        assert!(got.contains(want), "{event}: {got}");
    }
    // Columns count characters, not bytes.
    let got = error(&format!("{header}[0,\"o\",\"中\"x]\n"));
    assert!(got.starts_with("line 3, column 11:"), "{got}");
    // Huge times that stay finite are fine; nothing is timed.
    assert_eq!(decode_str(&format!("{header}[1e300,\"o\",\"!\"]\n")).unwrap().output, b"ok!");
    // Unknown codes, and any data on ignored events, are skipped.
    assert_eq!(decode_str(&format!("{header}[0,\"z\",{{}}]\n[0,\"i\",5]\n")).unwrap().output, b"ok");
}

#[test]
fn invalid_utf8_is_refused_with_its_place() {
    let mut data = b"{\"version\":2,\"width\":10,\"height\":2}\n[0,\"o\",\"ab".to_vec();
    data.extend_from_slice(b"\xff\"]\n");
    assert_eq!(decode(&data).unwrap_err(), "line 2, column 11: not UTF-8");
    // A truncated multibyte character, as a cut-off file ends.
    let mut cut = b"{\"version\":2,\"width\":10,\"height\":2}\n[0,\"o\",\"".to_vec();
    cut.extend_from_slice(&"中".as_bytes()[..2]);
    assert_eq!(decode(&cut).unwrap_err(), "line 2, column 9: not UTF-8");
    let header = b"{\"version\":2,\"width\":10,\"height\":2,\"title\":\"\xc3\"}";
    assert!(decode(header).unwrap_err().starts_with("line 1, column 45: not UTF-8"));
}

#[test]
fn truncated_casts_are_refused() {
    let whole = V2.as_bytes();
    // Cut anywhere inside a line, the cast is refused; cut after a whole
    // event, it reads that far. It never panics.
    for end in 0..whole.len() {
        let cut = &whole[..end];
        let at_line_end = end == 0 || whole[end - 1] == b'\n' || whole[end] == b'\n';
        match decode(cut) {
            Ok(cast) => assert!(at_line_end && end > 0, "cut at {end} read: {:?}", cast.output),
            Err(_) => assert!(!(at_line_end && end > V2.find('\n').unwrap())),
        }
    }
}
