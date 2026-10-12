use std::io::Cursor;

use export_sheet::{convert, convert_bytes, DelimReader, Format, JsonReader, Options};

fn bytes(from: Format, to: Format, input: &str) -> (String, export_sheet::Stats) {
    let (out, stats) = convert_bytes(from, to, input.as_bytes(), Options::default()).unwrap();
    (String::from_utf8(out).unwrap(), stats)
}

#[test]
fn csv_json_round_trip_keeps_cells() {
    let csv = "sku,name\nA-1,Colgate\nA-2,\"say \"\"hi\"\"\"\n";
    let (json, stats) = bytes(Format::Csv, Format::Json, csv);
    assert_eq!(stats.rows, 2);
    assert_eq!(stats.lossy, 0);
    assert_eq!(
        json,
        "[{\"sku\":\"A-1\",\"name\":\"Colgate\"},{\"sku\":\"A-2\",\"name\":\"say \\\"hi\\\"\"}]\n"
    );
    let (back, stats) = bytes(Format::Json, Format::Csv, &json);
    assert_eq!(stats.rows, 3);
    assert_eq!(back, csv);
}

#[test]
fn quoted_commas_and_embedded_newlines_are_one_row() {
    let csv = "sku,addr\nA-1,\"12 Mabini St, Makati\"\nA-2,\"Line one\nLine two\"\n";
    let (json, _) = bytes(Format::Csv, Format::Json, csv);
    assert!(json.contains("12 Mabini St, Makati"));
    assert!(json.contains("Line one\\nLine two"));
    let (back, _) = bytes(Format::Json, Format::Csv, &json);
    assert_eq!(back, csv);
}

#[test]
fn tsv_round_trip_quotes_a_tab_inside_a_cell() {
    let tsv = "sku\tname\nA-1\tColgate Total\nA-2\t\"has\ta tab\"\n";
    let (json, stats) = bytes(Format::Tsv, Format::Json, tsv);
    assert_eq!(stats.rows, 2);
    assert!(json.contains("has\\ta tab"));
    let (back, _) = bytes(Format::Json, Format::Tsv, &json);
    assert_eq!(back, tsv);
}

#[test]
fn csv_to_tsv_changes_the_delimiter_only() {
    let (tsv, stats) = bytes(Format::Csv, Format::Tsv, "a,b\n1,2\n");
    assert_eq!(stats.rows, 2);
    assert_eq!(tsv, "a\tb\n1\t2\n");
}

#[test]
fn a_ragged_row_is_an_error() {
    let err = convert_bytes(
        Format::Csv,
        Format::Json,
        b"sku,name\nA-1\n",
        Options::default(),
    )
    .unwrap_err();
    assert!(err.message().contains("columns"), "{err}");
}

#[test]
fn duplicate_column_names_are_an_error() {
    let err = convert_bytes(
        Format::Csv,
        Format::Json,
        b"sku,sku\nA-1,B\n",
        Options::default(),
    )
    .unwrap_err();
    assert!(err.message().contains("duplicate"), "{err}");
}

#[test]
fn blank_lines_are_skipped_and_a_tab_row_is_kept() {
    let (json, stats) = bytes(Format::Csv, Format::Json, "a,b\n\n   \n1,2\n");
    assert_eq!(stats.rows, 1);
    assert_eq!(json, "[{\"a\":\"1\",\"b\":\"2\"}]\n");

    let (json, stats) = bytes(Format::Tsv, Format::Json, "a\tb\n\t\n");
    assert_eq!(stats.rows, 1, "{json}");
    assert_eq!(json, "[{\"a\":\"\",\"b\":\"\"}]\n");
}

#[test]
fn header_is_trimmed_and_a_bom_is_ignored() {
    let mut raw = vec![0xEF, 0xBB, 0xBF];
    raw.extend_from_slice(b" sku , name \r\nA-1,Colgate\r\n");
    let (out, _) = convert_bytes(Format::Csv, Format::Json, &raw, Options::default()).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "[{\"sku\":\"A-1\",\"name\":\"Colgate\"}]\n"
    );
}

#[test]
fn invalid_utf8_is_repaired_and_counted_rather_than_fatal() {
    let mut raw = Vec::new();
    raw.extend_from_slice(b"sku,name\n");
    raw.extend_from_slice(b"A-1,caf\xe9 latte\n");
    raw.extend_from_slice(b"A-2,plain tea\n");

    let mut reader = DelimReader::new(Cursor::new(raw.clone()), b',');
    assert!(reader.next_row().unwrap());
    assert_eq!(reader.field(0), "sku");
    assert!(reader.next_row().unwrap());
    assert!(reader.field(1).contains('\u{FFFD}'));
    assert!(reader.field(1).contains("latte"));
    assert!(reader.next_row().unwrap());
    assert_eq!(reader.field(1), "plain tea");
    assert!(!reader.next_row().unwrap());
    assert_eq!(reader.lossy(), 1);

    let (out, stats) = convert_bytes(Format::Csv, Format::Json, &raw, Options::default()).unwrap();
    assert_eq!(stats.rows, 2);
    assert_eq!(stats.lossy, 1);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("plain tea"));
    assert!(text.contains('\u{FFFD}'));
}

#[test]
fn reader_keeps_one_row_not_the_file() {
    let mut csv = Vec::new();
    for i in 0..20_000 {
        csv.extend_from_slice(format!("r{i},name,ok\n").as_bytes());
    }
    let mut reader = DelimReader::new(Cursor::new(csv), b',');
    let mut n = 0;
    while reader.next_row().unwrap() {
        n += 1;
        assert!(
            reader.resident_capacity() < 64 * 1024,
            "capacity {}",
            reader.resident_capacity()
        );
    }
    assert_eq!(n, 20_000);
}

#[test]
fn json_scalars_and_the_refusal_of_nesting() {
    let (csv, _) = bytes(
        Format::Json,
        Format::Csv,
        "[{\"n\":1.50,\"ok\":true,\"missing\":null,\"note\":\"caf\\u00e9\"}]\n",
    );
    assert_eq!(csv, "n,ok,missing,note\n1.50,true,,café\n");

    let err = convert_bytes(
        Format::Json,
        Format::Csv,
        br#"[{"a":{"b":1}}]"#,
        Options::default(),
    )
    .unwrap_err();
    assert!(err.message().contains("nested"), "{err}");

    let err = convert_bytes(
        Format::Json,
        Format::Csv,
        br#"[{"a":"1"},{"a":"2","extra":"no"}]"#,
        Options::default(),
    )
    .unwrap_err();
    assert!(err.message().contains("extra"), "{err}");
}

#[test]
fn json_invalid_utf8_inside_a_string_does_not_drop_the_next_row() {
    let mut raw = Vec::new();
    raw.extend_from_slice(br#"[{"a":"caf"#);
    raw.push(0xFF);
    raw.extend_from_slice(br#""},{"a":"ok"}]"#);
    let (out, stats) = convert_bytes(Format::Json, Format::Csv, &raw, Options::default()).unwrap();
    assert_eq!(stats.lossy, 1);
    assert_eq!(stats.rows, 3);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("ok"));
    assert!(text.contains('\u{FFFD}'));
}

#[test]
fn json_reader_window_stays_bounded() {
    let mut json = Vec::from(&b"[\n"[..]);
    for i in 0..5_000 {
        if i > 0 {
            json.push(b',');
        }
        json.extend_from_slice(format!(r#"{{"a":"{i}","b":"x"}}"#).as_bytes());
    }
    json.extend_from_slice(b"]\n");
    let mut reader = JsonReader::new(Cursor::new(json));
    let mut n = 0;
    while reader.next_row().unwrap() {
        n += 1;
        assert!(
            reader.resident_capacity() < 256 * 1024,
            "capacity {}",
            reader.resident_capacity()
        );
    }
    assert_eq!(n, 5_000);
}

#[test]
fn empty_inputs_are_zero_rows() {
    let (json, stats) = bytes(Format::Csv, Format::Json, "");
    assert_eq!(json, "[]\n");
    assert_eq!(stats.rows, 0);
    let (csv, stats) = bytes(Format::Json, Format::Csv, "[]");
    assert_eq!(csv, "");
    assert_eq!(stats.rows, 0);
    let (json, stats) = bytes(Format::Csv, Format::Json, "a,b\n");
    assert_eq!(json, "[]\n");
    assert_eq!(stats.rows, 0);
}

#[test]
fn headerless_csv_is_an_array_of_arrays() {
    let (json, stats) = convert_bytes(
        Format::Csv,
        Format::Json,
        b"A-1,Colgate\n",
        Options { header: false },
    )
    .unwrap();
    assert_eq!(stats.rows, 1);
    assert_eq!(json, b"[[\"A-1\",\"Colgate\"]]\n");
}

#[test]
fn unterminated_quote_is_an_error() {
    let err = convert_bytes(Format::Csv, Format::Tsv, b"\"open\n", Options::default()).unwrap_err();
    assert!(err.message().contains("unterminated"), "{err}");
}

#[test]
fn surrogate_pair_decodes_to_one_character() {
    let (csv, stats) = bytes(Format::Json, Format::Csv, r#"[{"e":"\uD83D\uDE00"}]"#);
    assert_eq!(stats.lossy, 0);
    assert_eq!(csv, "e\n😀\n");
}

#[test]
fn streaming_convert_writes_without_a_second_buffer_of_rows() {
    let mut csv = Vec::from(&b"a,b\n"[..]);
    for i in 0..1_000 {
        csv.extend_from_slice(format!("{i},v\n").as_bytes());
    }
    let mut out = Vec::new();
    let stats = convert(
        Format::Csv,
        Format::Json,
        Cursor::new(csv),
        &mut out,
        Options::default(),
    )
    .unwrap();
    assert_eq!(stats.rows, 1_000);
    assert!(out.starts_with(b"[{"));
    assert!(out.ends_with(b"}]\n"));
}
