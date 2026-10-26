use export_sheet::{convert_bytes, Format, Options};

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn xlsx(csv: &str) -> Vec<u8> {
    convert_bytes(
        Format::Csv,
        Format::Xlsx,
        csv.as_bytes(),
        Options::default(),
    )
    .unwrap()
    .0
}

#[test]
fn csv_xlsx_round_trip_keeps_shared_strings_and_numbers() {
    let csv = "name,qty,note\napple,2,\"say \"\"hi\"\"\"\npie,3.50,a&b<c\n";
    let book = xlsx(csv);
    assert!(contains(&book, b"sharedStrings"), "shared string part");
    assert!(contains(&book, b"t=\"s\""), "string cells use the table");
    assert!(contains(&book, b"<v>2</v>"), "2 is a number");
    assert!(contains(&book, b"<v>3.50</v>"), "3.50 stays lexical on write");
    assert!(contains(&book, b"&amp;"), "shared strings escape &");
    let (back, stats) =
        convert_bytes(Format::Xlsx, Format::Csv, &book, Options::default()).unwrap();
    assert_eq!(stats.rows, 3);
    assert_eq!(stats.lossy, 0);
    // Reading casts "3.50" through a float, so the repr prints 3.5.
    assert_eq!(
        String::from_utf8(back).unwrap(),
        "name,qty,note\napple,2,\"say \"\"hi\"\"\"\npie,3.5,a&b<c\n"
    );
}

#[test]
fn leading_zero_stays_text_and_a_blank_cell_stays() {
    let csv = "zip,n\n007,1\n,2\n";
    let book = xlsx(csv);
    let (back, _) = convert_bytes(Format::Xlsx, Format::Csv, &book, Options::default()).unwrap();
    assert_eq!(String::from_utf8(back).unwrap(), csv);
}

#[test]
fn tsv_and_column_aa() {
    let mut header = vec!["h"; 27];
    header[26] = "last";
    let mut cells = vec!["x"; 27];
    cells[26] = "tail";
    let csv = format!("{}\n{}\n", header.join(","), cells.join(","));
    let book = xlsx(&csv);
    assert!(contains(&book, b"AA2"), "column 27 is AA");
    let (tsv, _) = convert_bytes(Format::Xlsx, Format::Tsv, &book, Options::default()).unwrap();
    let text = String::from_utf8(tsv).unwrap();
    assert!(text.ends_with("\ttail\n"), "{text}");
}

#[test]
fn a_ragged_csv_is_still_an_error() {
    let err =
        convert_bytes(Format::Csv, Format::Xlsx, b"a,b\n1\n", Options::default()).unwrap_err();
    assert!(err.message().contains("columns"), "{err}");
}

#[test]
fn json_is_not_an_xlsx_pair() {
    let err = convert_bytes(Format::Json, Format::Xlsx, b"[]", Options::default()).unwrap_err();
    assert!(err.message().contains("xlsx"), "{err}");
}

#[test]
fn one_sheet_only_reads_the_first() {
    let csv = "a\n1\n";
    let book = xlsx(csv);
    let (back, stats) =
        convert_bytes(Format::Xlsx, Format::Csv, &book, Options::default()).unwrap();
    assert_eq!(stats.rows, 2);
    assert_eq!(back, b"a\n1\n");
}
