use export_sheet::{
    convert_bytes, generated_biff_xls, generated_biff_xls_numeric_column,
    generated_biff_xls_sum_cache, generated_biff_xls_sum_uncached,
    generated_biff_xls_sum_uncached_three, generated_biff_xls_two_numbers, Format, Options,
};

#[test]
fn one_number_and_one_text() {
    let file = generated_biff_xls();
    assert_eq!(
        &file[..8],
        &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
    );
    let (csv, stats) = convert_bytes(Format::Xls, Format::Csv, &file, Options::default()).unwrap();
    assert_eq!(stats.rows, 1);
    assert_eq!(stats.lossy, 0);
    assert_eq!(csv, b"42,cat\n");
    let (tsv, _) = convert_bytes(Format::Xls, Format::Tsv, &file, Options::default()).unwrap();
    assert_eq!(tsv, b"42\tcat\n");
}

#[test]
fn two_numbers_and_the_same_text() {
    let file = generated_biff_xls_two_numbers();
    assert_eq!(
        &file[..8],
        &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
    );
    let (csv, stats) = convert_bytes(Format::Xls, Format::Csv, &file, Options::default()).unwrap();
    assert_eq!(stats.rows, 1);
    assert_eq!(stats.lossy, 0);
    assert_eq!(csv, b"42,cat,7\n");
}

#[test]
fn one_thousand_numeric_rows() {
    let file = generated_biff_xls_numeric_column();
    let (csv, stats) = convert_bytes(Format::Xls, Format::Csv, &file, Options::default()).unwrap();
    assert_eq!(stats.rows, 1000);
    assert_eq!(stats.lossy, 0);
    assert!(csv.starts_with(b"0\n"));
    assert!(csv.ends_with(b"999\n"));
    assert_eq!(csv.iter().filter(|b| **b == b'\n').count(), 1000);
}

#[test]
fn sum_formula_cached_value_is_five() {
    let file = generated_biff_xls_sum_cache();
    let (csv, stats) = convert_bytes(Format::Xls, Format::Csv, &file, Options::default()).unwrap();
    assert_eq!(stats.rows, 1);
    assert_eq!(csv, b"2,3,5\n");
}

#[test]
fn uncached_sum_is_seventeen() {
    let file = generated_biff_xls_sum_uncached();
    let (csv, stats) = convert_bytes(Format::Xls, Format::Csv, &file, Options::default()).unwrap();
    assert_eq!(stats.rows, 1);
    assert_eq!(csv, b"8,9,17\n");
}

#[test]
fn uncached_sum_of_three_is_fifteen() {
    let file = generated_biff_xls_sum_uncached_three();
    let (csv, stats) = convert_bytes(Format::Xls, Format::Csv, &file, Options::default()).unwrap();
    assert_eq!(stats.rows, 1);
    assert_eq!(csv, b"4,5,6,15\n");
}

#[test]
fn a_file_that_is_not_biff_is_refused() {
    let err =
        convert_bytes(Format::Xls, Format::Csv, b"not an xls", Options::default()).unwrap_err();
    assert!(err.message().contains("not BIFF"), "{err}");
}

#[test]
fn xls_is_not_a_json_pair() {
    let file = generated_biff_xls();
    let err = convert_bytes(Format::Xls, Format::Json, &file, Options::default()).unwrap_err();
    assert!(err.message().contains("xls"), "{err}");
    let err = convert_bytes(Format::Csv, Format::Xls, b"a\n1\n", Options::default()).unwrap_err();
    assert!(err.message().contains("xls"), "{err}");
}
