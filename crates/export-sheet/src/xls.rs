//! One generated BIFF8 workbook.
//!
//! The file is a version-3 compound document whose `Workbook` stream holds
//! BIFF8 records: a globals substream, one worksheet, one IEEE number, and
//! one shared string. A second generated file adds another IEEE number in
//! C1 and leaves B1 as that same string. Anything that is not that layout
//! is refused. There is
//! no writer for arbitrary sheets, and no guess for a record this parser
//! does not know how to read when that record is one of the ones below.

use std::io::{BufRead, Write};

use crate::delim;
use crate::error::{Error, Result};
use crate::Stats;

const MAX_FILE: usize = 64 * 1024 * 1024;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;
const FATSECT: u32 = 0xFFFF_FFFD;
const NOSTREAM: u32 = 0xFFFF_FFFF;

const NUMBER_VALUE: f64 = 42.0;
const TEXT_VALUE: &str = "cat";

/// The one `.xls` this crate reads: BIFF8, cell A1 = 42, cell B1 = `cat`.
pub fn generated_biff_xls() -> Vec<u8> {
    wrap_ole(&workbook_bytes())
}

/// Same BIFF8 layout as [`generated_biff_xls`], with a second number in C1.
/// A1 stays 42 and B1 stays `cat`.
pub fn generated_biff_xls_two_numbers() -> Vec<u8> {
    wrap_ole(&workbook_two_numbers())
}

/// One thousand rows, column A only, values 0 through 999. No formulas.
pub fn generated_biff_xls_numeric_column() -> Vec<u8> {
    wrap_ole_regular(&workbook_numeric_column())
}

/// The same numeric column at the next count: 2 000 rows, values 0 through 1999.
pub fn generated_biff_xls_numeric_column_two_thousand() -> Vec<u8> {
    wrap_ole_regular(&workbook_numeric_column_two_thousand())
}

/// The same numeric column at the next count after 2 000: 4 000 rows, values 0 through 3999.
pub fn generated_biff_xls_numeric_column_four_thousand() -> Vec<u8> {
    wrap_ole_two_fat(&workbook_numeric_column_four_thousand())
}

/// A1 = 2, B1 = 3, C1 = a SUM formula whose cached value is 5.
/// The expression is not evaluated.
pub fn generated_biff_xls_sum_cache() -> Vec<u8> {
    wrap_ole(&workbook_sum_cache())
}

/// A1 = 8, B1 = 9, C1 = SUM of those cells with no cached number.
pub fn generated_biff_xls_sum_uncached() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached())
}

/// A1 = 4, B1 = 5, C1 = 6, D1 = SUM of those three cells with no cached number.
pub fn generated_biff_xls_sum_uncached_three() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached_three())
}

/// A1 = 4, B1 = 5, C1 = 6, D1 = 7, E1 = SUM of those four cells with no cached number.
pub fn generated_biff_xls_sum_uncached_four() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached_four())
}

/// A1 = 1, B1 = 2, C1 = 3, D1 = SUM of the range A1:C1 with no cached number.
pub fn generated_biff_xls_sum_uncached_area() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached_area())
}

/// A1 = 1, B1 = 2, C1 = 3, D1 = 4, E1 = SUM of the range A1:D1 with no cached number.
pub fn generated_biff_xls_sum_uncached_area_four() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached_area_four())
}

/// A1 = 1, A2 = 2, A3 = 3, A4 = SUM of the column A1:A3 with no cached number.
pub fn generated_biff_xls_sum_uncached_area_column() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached_area_column())
}

/// A1 = 1, A2 = 2, A3 = 3, A4 = 4, A5 = SUM of the column A1:A4 with no cached number.
pub fn generated_biff_xls_sum_uncached_area_column_four() -> Vec<u8> {
    wrap_ole(&workbook_sum_uncached_area_column_four())
}

pub(crate) fn to_delim<R: BufRead, W: Write>(
    mut input: R,
    output: &mut W,
    delim: u8,
) -> Result<Stats> {
    let mut raw = Vec::new();
    input.read_to_end(&mut raw)?;
    if raw.len() > MAX_FILE {
        return Err(Error::new("xls is larger than 64MiB"));
    }
    let rows = read_biff8(&raw)?;
    for row in &rows {
        delim::write_row(output, delim, row.iter().map(String::as_str))?;
    }
    Ok(Stats {
        rows: rows.len() as u64,
        lossy: 0,
    })
}

fn workbook_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    push_rec(&mut out, 0x00FC, &sst(TEXT_VALUE));
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    // OpenOffice documents 0x04C1 as the usual worksheet WsBool.
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dimensions());
    push_rec(&mut out, 0x0203, &number_cell(0, 0, NUMBER_VALUE));
    push_rec(&mut out, 0x00FD, &labelsst_cell(0, 1, 0));
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn workbook_two_numbers() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    push_rec(&mut out, 0x00FC, &sst(TEXT_VALUE));
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&3u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, NUMBER_VALUE));
    push_rec(&mut out, 0x00FD, &labelsst_cell(0, 1, 0));
    push_rec(&mut out, 0x0203, &number_cell(0, 2, 7.0));
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn workbook_sum_cache() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&3u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 2.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 1, 3.0));
    push_rec(&mut out, 0x0006, &formula_sum_cached(0, 2, 5.0));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// BIFF8 FORMULA with a cached IEEE number. The rgce is SUM(A1,B1) and is not read back.
fn formula_sum_cached(row: u16, col: u16, cached: f64) -> Vec<u8> {
    let rgce = [
        0x44, 0x00, 0x00, 0x00, 0x00, // PtgRef value, A1
        0x44, 0x00, 0x00, 0x01, 0x00, // PtgRef value, B1
        0x42, 0x02, 0x04, 0x00, // PtgFuncVar value, 2 args, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&cached.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&3u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 8.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 1, 9.0));
    push_rec(&mut out, 0x0006, &formula_sum_uncached(0, 2));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// Same SUM(A1,B1) expression as [`formula_sum_cached`], with an empty formula value.
fn formula_sum_uncached(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x44, 0x00, 0x00, 0x00, 0x00, // PtgRef value, A1
        0x44, 0x00, 0x00, 0x01, 0x00, // PtgRef value, B1
        0x42, 0x02, 0x04, 0x00, // PtgFuncVar value, 2 args, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached_three() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&4u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 4.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 1, 5.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 2, 6.0));
    push_rec(&mut out, 0x0006, &formula_sum_uncached_three(0, 3));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// SUM(A1,B1,C1) with an empty formula value. The token list is this shape only.
fn formula_sum_uncached_three(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x44, 0x00, 0x00, 0x00, 0x00, // PtgRef value, A1
        0x44, 0x00, 0x00, 0x01, 0x00, // PtgRef value, B1
        0x44, 0x00, 0x00, 0x02, 0x00, // PtgRef value, C1
        0x42, 0x03, 0x04, 0x00, // PtgFuncVar value, 3 args, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached_four() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&5u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 4.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 1, 5.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 2, 6.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 3, 7.0));
    push_rec(&mut out, 0x0006, &formula_sum_uncached_four(0, 4));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// SUM(A1,B1,C1,D1) with an empty formula value. The token list is this shape only.
fn formula_sum_uncached_four(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x44, 0x00, 0x00, 0x00, 0x00, // PtgRef value, A1
        0x44, 0x00, 0x00, 0x01, 0x00, // PtgRef value, B1
        0x44, 0x00, 0x00, 0x02, 0x00, // PtgRef value, C1
        0x44, 0x00, 0x00, 0x03, 0x00, // PtgRef value, D1
        0x42, 0x04, 0x04, 0x00, // PtgFuncVar value, 4 args, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached_area() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&4u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 1.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 1, 2.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 2, 3.0));
    push_rec(&mut out, 0x0006, &formula_sum_uncached_area(0, 3));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// SUM(A1:C1) with an empty formula value. One area token, then SUM. Nothing else.
fn formula_sum_uncached_area(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x45, // PtgArea value
        0x00, 0x00, // first row
        0x00, 0x00, // last row
        0x00, 0x00, // first column, A
        0x02, 0x00, // last column, C
        0x42, 0x01, 0x04, 0x00, // PtgFuncVar value, 1 arg, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached_area_four() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&5u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 1.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 1, 2.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 2, 3.0));
    push_rec(&mut out, 0x0203, &number_cell(0, 3, 4.0));
    push_rec(&mut out, 0x0006, &formula_sum_uncached_area_four(0, 4));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// SUM(A1:D1) with an empty formula value. The same area token as A1:C1, one column wider.
fn formula_sum_uncached_area_four(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x45, // PtgArea value
        0x00, 0x00, // first row
        0x00, 0x00, // last row
        0x00, 0x00, // first column, A
        0x03, 0x00, // last column, D
        0x42, 0x01, 0x04, 0x00, // PtgFuncVar value, 1 arg, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached_area_column() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[4..8].copy_from_slice(&4u32.to_le_bytes());
    dims[10..12].copy_from_slice(&1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 1.0));
    push_rec(&mut out, 0x0203, &number_cell(1, 0, 2.0));
    push_rec(&mut out, 0x0203, &number_cell(2, 0, 3.0));
    push_rec(&mut out, 0x0006, &formula_sum_uncached_area_column(3, 0));
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// SUM(A1:A3) with an empty formula value. The same area token, running down column A.
fn formula_sum_uncached_area_column(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x45, // PtgArea value
        0x00, 0x00, // first row, 1
        0x02, 0x00, // last row, 3
        0x00, 0x00, // first column, A
        0x00, 0x00, // last column, A
        0x42, 0x01, 0x04, 0x00, // PtgFuncVar value, 1 arg, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_sum_uncached_area_column_four() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[4..8].copy_from_slice(&5u32.to_le_bytes());
    dims[10..12].copy_from_slice(&1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, 1.0));
    push_rec(&mut out, 0x0203, &number_cell(1, 0, 2.0));
    push_rec(&mut out, 0x0203, &number_cell(2, 0, 3.0));
    push_rec(&mut out, 0x0203, &number_cell(3, 0, 4.0));
    push_rec(
        &mut out,
        0x0006,
        &formula_sum_uncached_area_column_four(4, 0),
    );
    push_rec(&mut out, 0x000A, &[]);
    out
}

/// SUM(A1:A4) with an empty formula value. The A1:A3 area token, one row longer.
fn formula_sum_uncached_area_column_four(row: u16, col: u16) -> Vec<u8> {
    let rgce = [
        0x45, // PtgArea value
        0x00, 0x00, // first row, 1
        0x03, 0x00, // last row, 4
        0x00, 0x00, // first column, A
        0x00, 0x00, // last column, A
        0x42, 0x01, 0x04, 0x00, // PtgFuncVar value, 1 arg, SUM
    ];
    let mut data = Vec::with_capacity(22 + rgce.len());
    data.extend_from_slice(&row.to_le_bytes());
    data.extend_from_slice(&col.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    data.extend_from_slice(&rgce);
    data
}

fn workbook_numeric_column() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = [0u8; 14];
    dims[4..8].copy_from_slice(&1000u32.to_le_bytes());
    dims[10..12].copy_from_slice(&1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    for row in 0..1000u16 {
        push_rec(&mut out, 0x0203, &number_cell(row, 0, f64::from(row)));
    }
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn workbook_numeric_column_two_thousand() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = [0u8; 14];
    dims[4..8].copy_from_slice(&2000u32.to_le_bytes());
    dims[10..12].copy_from_slice(&1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    for row in 0..2000u16 {
        push_rec(&mut out, 0x0203, &number_cell(row, 0, f64::from(row)));
    }
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn workbook_numeric_column_four_thousand() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = [0u8; 14];
    dims[4..8].copy_from_slice(&4000u32.to_le_bytes());
    dims[10..12].copy_from_slice(&1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    for row in 0..4000u16 {
        push_rec(&mut out, 0x0203, &number_cell(row, 0, f64::from(row)));
    }
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn bof(dt: u16) -> [u8; 16] {
    let mut data = [0u8; 16];
    data[0..2].copy_from_slice(&0x0600u16.to_le_bytes());
    data[2..4].copy_from_slice(&dt.to_le_bytes());
    data[4..6].copy_from_slice(&0x0DBBu16.to_le_bytes());
    data[6..8].copy_from_slice(&0x07CCu16.to_le_bytes());
    data[8..12].copy_from_slice(&0x0000_00C1u32.to_le_bytes());
    data[12..16].copy_from_slice(&0x0000_0006u32.to_le_bytes());
    data
}

fn sst(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut data = Vec::with_capacity(8 + 3 + bytes.len());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    data.push(0);
    data.extend_from_slice(bytes);
    data
}

fn boundsheet(pos: u32, name: &str) -> Vec<u8> {
    let bytes = name.as_bytes();
    let mut data = Vec::with_capacity(8 + bytes.len());
    data.extend_from_slice(&pos.to_le_bytes());
    // hsState is the low 2 bits. dt (worksheet = 0) is the high byte.
    data.extend_from_slice(&0u16.to_le_bytes());
    data.push(bytes.len() as u8);
    data.push(0);
    data.extend_from_slice(bytes);
    data
}

fn dimensions() -> [u8; 14] {
    let mut data = [0u8; 14];
    data[4..8].copy_from_slice(&1u32.to_le_bytes());
    data[10..12].copy_from_slice(&2u16.to_le_bytes());
    data
}

fn number_cell(row: u16, col: u16, value: f64) -> [u8; 14] {
    let mut data = [0u8; 14];
    data[0..2].copy_from_slice(&row.to_le_bytes());
    data[2..4].copy_from_slice(&col.to_le_bytes());
    data[6..14].copy_from_slice(&value.to_le_bytes());
    data
}

fn labelsst_cell(row: u16, col: u16, index: u32) -> [u8; 10] {
    let mut data = [0u8; 10];
    data[0..2].copy_from_slice(&row.to_le_bytes());
    data[2..4].copy_from_slice(&col.to_le_bytes());
    data[6..10].copy_from_slice(&index.to_le_bytes());
    data
}

fn push_rec(out: &mut Vec<u8>, op: u16, data: &[u8]) {
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(data);
}

fn wrap_ole(workbook: &[u8]) -> Vec<u8> {
    let mini = 64usize;
    let minis = workbook.len().div_ceil(mini);
    let mini_stream_size = minis * mini;
    let mut file = vec![0u8; 512 * 5];
    file[..8].copy_from_slice(&OLE_MAGIC);
    put_u16(&mut file, 0x18, 0x003E);
    put_u16(&mut file, 0x1A, 0x0003);
    put_u16(&mut file, 0x1C, 0xFFFE);
    put_u16(&mut file, 0x1E, 9);
    put_u16(&mut file, 0x20, 6);
    put_u32(&mut file, 0x2C, 1);
    put_u32(&mut file, 0x30, 1);
    put_u32(&mut file, 0x38, 4096);
    put_u32(&mut file, 0x3C, 2);
    put_u32(&mut file, 0x40, 1);
    put_u32(&mut file, 0x44, ENDOFCHAIN);
    put_u32(&mut file, 0x4C, 0);
    for i in 1..109 {
        put_u32(&mut file, 0x4C + i * 4, FREESECT);
    }

    let fat = 512;
    for i in 0..128 {
        put_u32(&mut file, fat + i * 4, FREESECT);
    }
    put_u32(&mut file, fat, FATSECT);
    put_u32(&mut file, fat + 4, ENDOFCHAIN);
    put_u32(&mut file, fat + 8, ENDOFCHAIN);
    put_u32(&mut file, fat + 12, ENDOFCHAIN);

    let dir = 1024;
    write_dir(
        &mut file[dir..dir + 128],
        "Root Entry",
        5,
        1,
        3,
        mini_stream_size as u64,
    );
    write_dir(
        &mut file[dir + 128..dir + 256],
        "Workbook",
        2,
        NOSTREAM,
        0,
        workbook.len() as u64,
    );
    free_dir(&mut file[dir + 256..dir + 384]);
    free_dir(&mut file[dir + 384..dir + 512]);

    let minifat = 1536;
    for i in 0..128 {
        put_u32(&mut file, minifat + i * 4, FREESECT);
    }
    for i in 0..minis {
        let next = if i + 1 == minis {
            ENDOFCHAIN
        } else {
            (i + 1) as u32
        };
        put_u32(&mut file, minifat + i * 4, next);
    }
    file[2048..2048 + workbook.len()].copy_from_slice(workbook);
    file
}

/// A Workbook stream of at least 4096 bytes lives in regular FAT sectors.
/// The small generated files stay on the mini-stream writer above.
fn wrap_ole_regular(workbook: &[u8]) -> Vec<u8> {
    let data_sectors = workbook.len().div_ceil(512);
    let total_sectors = 2 + data_sectors;
    let mut file = vec![0u8; 512 * (1 + total_sectors)];
    file[..8].copy_from_slice(&OLE_MAGIC);
    put_u16(&mut file, 0x18, 0x003E);
    put_u16(&mut file, 0x1A, 0x0003);
    put_u16(&mut file, 0x1C, 0xFFFE);
    put_u16(&mut file, 0x1E, 9);
    put_u16(&mut file, 0x20, 6);
    put_u32(&mut file, 0x2C, 1);
    put_u32(&mut file, 0x30, 1);
    put_u32(&mut file, 0x38, 4096);
    put_u32(&mut file, 0x3C, ENDOFCHAIN);
    put_u32(&mut file, 0x40, 0);
    put_u32(&mut file, 0x44, ENDOFCHAIN);
    put_u32(&mut file, 0x4C, 0);
    for i in 1..109 {
        put_u32(&mut file, 0x4C + i * 4, FREESECT);
    }

    let fat = 512;
    for i in 0..128 {
        put_u32(&mut file, fat + i * 4, FREESECT);
    }
    put_u32(&mut file, fat, FATSECT);
    put_u32(&mut file, fat + 4, ENDOFCHAIN);
    for i in 0..data_sectors {
        let sid = 2 + i;
        let next = if i + 1 == data_sectors {
            ENDOFCHAIN
        } else {
            (sid + 1) as u32
        };
        put_u32(&mut file, fat + sid * 4, next);
    }

    let dir = 1024;
    write_dir(&mut file[dir..dir + 128], "Root Entry", 5, 1, ENDOFCHAIN, 0);
    write_dir(
        &mut file[dir + 128..dir + 256],
        "Workbook",
        2,
        NOSTREAM,
        2,
        workbook.len() as u64,
    );
    free_dir(&mut file[dir + 256..dir + 384]);
    free_dir(&mut file[dir + 384..dir + 512]);
    file[1536..1536 + workbook.len()].copy_from_slice(workbook);
    file
}

/// Two FAT sectors, both named in the header. `num_difat` stays 0.
/// The 1 000-row and 2 000-row files stay on [`wrap_ole_regular`].
fn wrap_ole_two_fat(workbook: &[u8]) -> Vec<u8> {
    let data_sectors = workbook.len().div_ceil(512);
    // Sector 0 and 1 are the FAT. Sector 2 is the directory. Data starts at 3.
    let total_sectors = 3 + data_sectors;
    let mut file = vec![0u8; 512 * (1 + total_sectors)];
    file[..8].copy_from_slice(&OLE_MAGIC);
    put_u16(&mut file, 0x18, 0x003E);
    put_u16(&mut file, 0x1A, 0x0003);
    put_u16(&mut file, 0x1C, 0xFFFE);
    put_u16(&mut file, 0x1E, 9);
    put_u16(&mut file, 0x20, 6);
    put_u32(&mut file, 0x2C, 2);
    put_u32(&mut file, 0x30, 2);
    put_u32(&mut file, 0x38, 4096);
    put_u32(&mut file, 0x3C, ENDOFCHAIN);
    put_u32(&mut file, 0x40, 0);
    put_u32(&mut file, 0x44, ENDOFCHAIN);
    put_u32(&mut file, 0x4C, 0);
    put_u32(&mut file, 0x50, 1);
    for i in 2..109 {
        put_u32(&mut file, 0x4C + i * 4, FREESECT);
    }

    for i in 0..256 {
        put_u32(&mut file, 512 + i * 4, FREESECT);
    }
    put_u32(&mut file, 512, FATSECT);
    put_u32(&mut file, 512 + 4, FATSECT);
    put_u32(&mut file, 512 + 8, ENDOFCHAIN);
    for i in 0..data_sectors {
        let sid = 3 + i;
        let next = if i + 1 == data_sectors {
            ENDOFCHAIN
        } else {
            (sid + 1) as u32
        };
        put_u32(&mut file, 512 + sid * 4, next);
    }

    let dir = 1536;
    write_dir(&mut file[dir..dir + 128], "Root Entry", 5, 1, ENDOFCHAIN, 0);
    write_dir(
        &mut file[dir + 128..dir + 256],
        "Workbook",
        2,
        NOSTREAM,
        3,
        workbook.len() as u64,
    );
    free_dir(&mut file[dir + 256..dir + 384]);
    free_dir(&mut file[dir + 384..dir + 512]);
    file[2048..2048 + workbook.len()].copy_from_slice(workbook);
    file
}

fn write_dir(entry: &mut [u8], name: &str, kind: u8, child: u32, start: u32, size: u64) {
    let mut n = 0;
    for unit in name.encode_utf16() {
        entry[n..n + 2].copy_from_slice(&unit.to_le_bytes());
        n += 2;
    }
    entry[0x40..0x42].copy_from_slice(&((n + 2) as u16).to_le_bytes());
    entry[0x42] = kind;
    entry[0x43] = 1;
    entry[0x44..0x48].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x48..0x4C].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x4C..0x50].copy_from_slice(&child.to_le_bytes());
    entry[0x74..0x78].copy_from_slice(&start.to_le_bytes());
    entry[0x78..0x80].copy_from_slice(&size.to_le_bytes());
}

fn free_dir(entry: &mut [u8]) {
    entry[0x44..0x48].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x48..0x4C].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x4C..0x50].copy_from_slice(&NOSTREAM.to_le_bytes());
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_biff8(file: &[u8]) -> Result<Vec<Vec<String>>> {
    if file.len() < 512 || file[..8] != OLE_MAGIC {
        return Err(Error::new("xls is not BIFF"));
    }
    let stream = workbook_stream(file)?;
    parse_workbook(&stream)
}

fn workbook_stream(file: &[u8]) -> Result<Vec<u8>> {
    let major = u16_at(file, 0x1A)?;
    let order = u16_at(file, 0x1C)?;
    let sec_shift = u16_at(file, 0x1E)?;
    let mini_shift = u16_at(file, 0x20)?;
    if order != 0xFFFE || major != 3 || sec_shift != 9 || mini_shift != 6 {
        return Err(Error::new("xls is not BIFF"));
    }
    let num_fat = u32_at(file, 0x2C)?;
    let dir_start = u32_at(file, 0x30)?;
    let cutoff = u32_at(file, 0x38)?;
    let minifat_start = u32_at(file, 0x3C)?;
    let num_minifat = u32_at(file, 0x40)?;
    let num_difat = u32_at(file, 0x48)?;
    if cutoff != 4096 || num_difat != 0 || num_fat == 0 || num_fat > 109 {
        return Err(Error::new("xls is not BIFF"));
    }
    let mut fat = Vec::new();
    for i in 0..num_fat as usize {
        let sid = u32_at(file, 0x4C + i * 4)?;
        fat.extend_from_slice(&sector(file, sid)?);
    }
    let directory = read_fat_chain(file, &fat, dir_start, None)?;
    if directory.len() < 256 || entry_name(&directory[..128])? != "Root Entry" {
        return Err(Error::new("xls is not BIFF"));
    }
    let workbook = directory
        .chunks(128)
        .skip(1)
        .find(|entry| entry[0x42] == 2 && entry_name(entry).ok().as_deref() == Some("Workbook"))
        .ok_or_else(|| Error::new("xls is not BIFF"))?;
    let start = u32::from_le_bytes(workbook[0x74..0x78].try_into().unwrap());
    let size = u32::from_le_bytes(workbook[0x78..0x7C].try_into().unwrap()) as usize;
    if size > MAX_FILE {
        return Err(Error::new("xls is larger than 64MiB"));
    }
    if (size as u32) < cutoff {
        let mini_stream_size = u32::from_le_bytes(directory[0x78..0x7C].try_into().unwrap());
        let root_start = u32::from_le_bytes(directory[0x74..0x78].try_into().unwrap());
        let mini_stream = read_fat_chain(file, &fat, root_start, Some(mini_stream_size))?;
        let minifat = read_fat_chain(
            file,
            &fat,
            minifat_start,
            Some(num_minifat.saturating_mul(512)),
        )?;
        return read_mini_chain(&mini_stream, &minifat, start, size);
    }
    read_fat_chain(file, &fat, start, Some(size as u32))
}

fn parse_workbook(stream: &[u8]) -> Result<Vec<Vec<String>>> {
    if stream.len() < 8 {
        return Err(Error::new("xls is not BIFF"));
    }
    let mut i = 0usize;
    let mut saw_globals = false;
    let mut in_sheet = false;
    let mut sheet_done = false;
    let mut strings: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut sheet_at: Option<u32> = None;
    let mut saw_sheet_bof = false;
    while i + 4 <= stream.len() {
        let op = u16::from_le_bytes(stream[i..i + 2].try_into().unwrap());
        let len = u16::from_le_bytes(stream[i + 2..i + 4].try_into().unwrap()) as usize;
        let data_at = i + 4;
        let next = data_at
            .checked_add(len)
            .ok_or_else(|| Error::new("xls record is truncated"))?;
        if next > stream.len() {
            return Err(Error::new("xls record is truncated"));
        }
        let data = &stream[data_at..next];
        if !saw_globals && op != 0x0809 {
            return Err(Error::new("xls is not BIFF"));
        }
        match op {
            0x0809 => {
                if data.len() != 16 {
                    return Err(Error::new("xls is not BIFF"));
                }
                let vers = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let dt = u16::from_le_bytes(data[2..4].try_into().unwrap());
                if vers != 0x0600 {
                    return Err(Error::new("xls is not BIFF"));
                }
                if !saw_globals {
                    if dt != 0x0005 {
                        return Err(Error::new("xls is not BIFF"));
                    }
                    saw_globals = true;
                } else if dt == 0x0010 && !sheet_done {
                    in_sheet = true;
                    saw_sheet_bof = true;
                } else if in_sheet {
                    return Err(Error::new("xls sheet bof is nested"));
                }
            }
            0x000A => {
                if len != 0 {
                    return Err(Error::new("xls eof length is refused"));
                }
                if in_sheet {
                    in_sheet = false;
                    sheet_done = true;
                }
            }
            0x002F => return Err(Error::new("encrypted xls is refused")),
            0x0085 if sheet_at.is_none() => {
                if data.len() < 4 {
                    return Err(Error::new("xls boundsheet is truncated"));
                }
                sheet_at = Some(u32::from_le_bytes(data[0..4].try_into().unwrap()));
            }
            0x00FC if !in_sheet => strings = parse_sst(data)?,
            0x003C => return Err(Error::new("xls continue record is refused")),
            0x0203 if in_sheet => {
                if data.len() != 14 {
                    return Err(Error::new("xls number record length is refused"));
                }
                let row = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let col = u16::from_le_bytes(data[2..4].try_into().unwrap());
                let value = f64::from_le_bytes(data[6..14].try_into().unwrap());
                place(&mut rows, row, col, number_text(value)?)?;
            }
            0x0006 if in_sheet => {
                if data.len() < 14 {
                    return Err(Error::new("xls formula record has no cached number"));
                }
                let marker = u16::from_le_bytes(data[12..14].try_into().unwrap());
                let row = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let col = u16::from_le_bytes(data[2..4].try_into().unwrap());
                let value = if marker == 0xFFFF {
                    sum_uncached_numeric_cells(data, &rows)?
                } else {
                    f64::from_le_bytes(data[6..14].try_into().unwrap())
                };
                place(&mut rows, row, col, number_text(value)?)?;
            }
            0x00FD if in_sheet => {
                if data.len() != 10 {
                    return Err(Error::new("xls labelsst record length is refused"));
                }
                let row = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let col = u16::from_le_bytes(data[2..4].try_into().unwrap());
                let index = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
                let text = strings.get(index).cloned().ok_or_else(|| {
                    Error::new(format!("xls shared string index {index} is out of range"))
                })?;
                place(&mut rows, row, col, text)?;
            }
            _ => {}
        }
        i = next;
    }
    if !saw_globals || !saw_sheet_bof {
        return Err(Error::new("xls is not BIFF"));
    }
    let pos = sheet_at.ok_or_else(|| Error::new("xls is not BIFF"))? as usize;
    if stream.get(pos..pos + 4) != Some(&[0x09, 0x08, 0x10, 0x00]) {
        return Err(Error::new("xls is not BIFF"));
    }
    Ok(rows)
}

fn parse_sst(data: &[u8]) -> Result<Vec<String>> {
    if data.len() < 8 {
        return Err(Error::new("xls shared string table is truncated"));
    }
    let total = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let unique = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if unique > total || unique > 1_048_576 {
        return Err(Error::new("xls shared string table is refused"));
    }
    let mut i = 8usize;
    let mut out = Vec::with_capacity(unique as usize);
    for _ in 0..unique {
        if i + 3 > data.len() {
            return Err(Error::new("xls shared string table is truncated"));
        }
        let chars = u16::from_le_bytes(data[i..i + 2].try_into().unwrap()) as usize;
        let flags = data[i + 2];
        i += 3;
        if flags & !1 != 0 {
            return Err(Error::new("xls shared string flags are refused"));
        }
        let nbytes = if flags & 1 == 0 {
            chars
        } else {
            chars
                .checked_mul(2)
                .ok_or_else(|| Error::new("xls shared string is refused"))?
        };
        if i + nbytes > data.len() {
            return Err(Error::new("xls shared string table is truncated"));
        }
        let text = if flags & 1 == 0 {
            data[i..i + nbytes].iter().map(|&b| char::from(b)).collect()
        } else {
            let units: Vec<u16> = data[i..i + nbytes]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).map_err(|_| Error::new("xls shared string is not utf-16"))?
        };
        i += nbytes;
        out.push(text);
    }
    if i != data.len() {
        return Err(Error::new("xls shared string table has trailing bytes"));
    }
    Ok(out)
}

/// Uncached SUM of two, three, or four cell refs, or one three- or four-cell area. Anything else stays refused.
fn sum_uncached_numeric_cells(data: &[u8], rows: &[Vec<String>]) -> Result<f64> {
    if data.len() < 22 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let cce = u16::from_le_bytes(data[20..22].try_into().unwrap()) as usize;
    if cce == 13 {
        sum_three_cell_range(data, rows)
    } else if cce == 14 {
        sum_two_numeric_cells(data, rows)
    } else if cce == 19 {
        sum_three_numeric_cells(data, rows)
    } else if cce == 24 {
        sum_four_numeric_cells(data, rows)
    } else {
        Err(Error::new("xls formula record has no cached number"))
    }
}

/// SUM of two value-class cell refs, and nothing else. Other expressions stay refused.
fn sum_two_numeric_cells(data: &[u8], rows: &[Vec<String>]) -> Result<f64> {
    if data.len() < 22 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let cce = u16::from_le_bytes(data[20..22].try_into().unwrap()) as usize;
    if data.len() != 22 + cce || cce != 14 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let rgce = &data[22..];
    if rgce[0] != 0x44 || rgce[5] != 0x44 || rgce[10] != 0x42 || rgce[11] != 2 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    if u16::from_le_bytes(rgce[12..14].try_into().unwrap()) != 4 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let left = cell_number(
        rows,
        u16::from_le_bytes(rgce[1..3].try_into().unwrap()),
        u16::from_le_bytes(rgce[3..5].try_into().unwrap()),
    )?;
    let right = cell_number(
        rows,
        u16::from_le_bytes(rgce[6..8].try_into().unwrap()),
        u16::from_le_bytes(rgce[8..10].try_into().unwrap()),
    )?;
    let sum = left + right;
    if !sum.is_finite() {
        return Err(Error::new("xls sum is not finite"));
    }
    Ok(sum)
}

/// SUM of three value-class cell refs, and nothing else. Other expressions stay refused.
fn sum_three_numeric_cells(data: &[u8], rows: &[Vec<String>]) -> Result<f64> {
    if data.len() < 22 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let cce = u16::from_le_bytes(data[20..22].try_into().unwrap()) as usize;
    if data.len() != 22 + cce || cce != 19 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let rgce = &data[22..];
    if rgce[0] != 0x44 || rgce[5] != 0x44 || rgce[10] != 0x44 || rgce[15] != 0x42 || rgce[16] != 3 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    if u16::from_le_bytes(rgce[17..19].try_into().unwrap()) != 4 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let first = cell_number(
        rows,
        u16::from_le_bytes(rgce[1..3].try_into().unwrap()),
        u16::from_le_bytes(rgce[3..5].try_into().unwrap()),
    )?;
    let second = cell_number(
        rows,
        u16::from_le_bytes(rgce[6..8].try_into().unwrap()),
        u16::from_le_bytes(rgce[8..10].try_into().unwrap()),
    )?;
    let third = cell_number(
        rows,
        u16::from_le_bytes(rgce[11..13].try_into().unwrap()),
        u16::from_le_bytes(rgce[13..15].try_into().unwrap()),
    )?;
    let sum = first + second + third;
    if !sum.is_finite() {
        return Err(Error::new("xls sum is not finite"));
    }
    Ok(sum)
}

/// SUM of four value-class cell refs, and nothing else. Other expressions stay refused.
fn sum_four_numeric_cells(data: &[u8], rows: &[Vec<String>]) -> Result<f64> {
    if data.len() < 22 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let cce = u16::from_le_bytes(data[20..22].try_into().unwrap()) as usize;
    if data.len() != 22 + cce || cce != 24 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let rgce = &data[22..];
    if rgce[0] != 0x44
        || rgce[5] != 0x44
        || rgce[10] != 0x44
        || rgce[15] != 0x44
        || rgce[20] != 0x42
        || rgce[21] != 4
    {
        return Err(Error::new("xls formula record has no cached number"));
    }
    if u16::from_le_bytes(rgce[22..24].try_into().unwrap()) != 4 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let first = cell_number(
        rows,
        u16::from_le_bytes(rgce[1..3].try_into().unwrap()),
        u16::from_le_bytes(rgce[3..5].try_into().unwrap()),
    )?;
    let second = cell_number(
        rows,
        u16::from_le_bytes(rgce[6..8].try_into().unwrap()),
        u16::from_le_bytes(rgce[8..10].try_into().unwrap()),
    )?;
    let third = cell_number(
        rows,
        u16::from_le_bytes(rgce[11..13].try_into().unwrap()),
        u16::from_le_bytes(rgce[13..15].try_into().unwrap()),
    )?;
    let fourth = cell_number(
        rows,
        u16::from_le_bytes(rgce[16..18].try_into().unwrap()),
        u16::from_le_bytes(rgce[18..20].try_into().unwrap()),
    )?;
    let sum = first + second + third + fourth;
    if !sum.is_finite() {
        return Err(Error::new("xls sum is not finite"));
    }
    Ok(sum)
}

/// SUM of one area that is a line of three or four cells. Other expressions stay refused.
fn sum_three_cell_range(data: &[u8], rows: &[Vec<String>]) -> Result<f64> {
    if data.len() < 22 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let cce = u16::from_le_bytes(data[20..22].try_into().unwrap()) as usize;
    if data.len() != 22 + cce || cce != 13 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let rgce = &data[22..];
    if rgce[0] != 0x45 || rgce[9] != 0x42 || rgce[10] != 1 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    if u16::from_le_bytes(rgce[11..13].try_into().unwrap()) != 4 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    let rw_first = u16::from_le_bytes(rgce[1..3].try_into().unwrap());
    let rw_last = u16::from_le_bytes(rgce[3..5].try_into().unwrap());
    let col_first = u16::from_le_bytes(rgce[5..7].try_into().unwrap());
    let col_last = u16::from_le_bytes(rgce[7..9].try_into().unwrap());
    let sum = if let Ok(cells) = three_cell_line(rw_first, rw_last, col_first, col_last) {
        add_line(rows, &cells)?
    } else if let Ok(cells) = four_cell_line(rw_first, rw_last, col_first, col_last) {
        add_line(rows, &cells)?
    } else {
        return Err(Error::new("xls formula record has no cached number"));
    };
    if !sum.is_finite() {
        return Err(Error::new("xls sum is not finite"));
    }
    Ok(sum)
}

fn add_line(rows: &[Vec<String>], cells: &[(u16, u16)]) -> Result<f64> {
    let mut sum = 0.0;
    for &(row, col) in cells {
        sum += cell_number(rows, row, col)?;
    }
    Ok(sum)
}

/// A straight line of three cells. A rectangle, or any other length, is refused.
fn three_cell_line(
    rw_first: u16,
    rw_last: u16,
    col_first: u16,
    col_last: u16,
) -> Result<[(u16, u16); 3]> {
    if col_first > 255 || col_last > 255 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    if rw_first == rw_last && u32::from(col_last) == u32::from(col_first) + 2 {
        return Ok([
            (rw_first, col_first),
            (rw_first, col_first + 1),
            (rw_first, col_first + 2),
        ]);
    }
    if col_first == col_last && u32::from(rw_last) == u32::from(rw_first) + 2 {
        return Ok([
            (rw_first, col_first),
            (rw_first + 1, col_first),
            (rw_first + 2, col_first),
        ]);
    }
    Err(Error::new("xls formula record has no cached number"))
}

/// A straight line of four cells. A rectangle, or any other length, is refused.
fn four_cell_line(
    rw_first: u16,
    rw_last: u16,
    col_first: u16,
    col_last: u16,
) -> Result<[(u16, u16); 4]> {
    if col_first > 255 || col_last > 255 {
        return Err(Error::new("xls formula record has no cached number"));
    }
    if rw_first == rw_last && u32::from(col_last) == u32::from(col_first) + 3 {
        return Ok([
            (rw_first, col_first),
            (rw_first, col_first + 1),
            (rw_first, col_first + 2),
            (rw_first, col_first + 3),
        ]);
    }
    if col_first == col_last && u32::from(rw_last) == u32::from(rw_first) + 3 {
        return Ok([
            (rw_first, col_first),
            (rw_first + 1, col_first),
            (rw_first + 2, col_first),
            (rw_first + 3, col_first),
        ]);
    }
    Err(Error::new("xls formula record has no cached number"))
}

fn cell_number(rows: &[Vec<String>], row: u16, col: u16) -> Result<f64> {
    if col > 255 {
        return Err(Error::new("xls sum cell is not a number"));
    }
    let text = rows
        .get(row as usize)
        .and_then(|line| line.get(col as usize))
        .ok_or_else(|| Error::new("xls sum cell is missing"))?;
    let value: f64 = text
        .parse()
        .map_err(|_| Error::new("xls sum cell is not a number"))?;
    if !value.is_finite() {
        return Err(Error::new("xls sum cell is not a number"));
    }
    Ok(value)
}

fn number_text(value: f64) -> Result<String> {
    if !value.is_finite() {
        return Err(Error::new("xls number is not finite"));
    }
    Ok(format!("{value}"))
}

fn place(rows: &mut Vec<Vec<String>>, row: u16, col: u16, text: String) -> Result<()> {
    if col > 255 {
        return Err(Error::new("xls cell is outside BIFF8 limits"));
    }
    let row = row as usize;
    let col = col as usize;
    while rows.len() < row {
        rows.push(Vec::new());
    }
    if rows.len() == row {
        rows.push(Vec::new());
    }
    let line = &mut rows[row];
    if col < line.len() {
        return Err(Error::new("xls cells are out of order"));
    }
    while line.len() < col {
        line.push(String::new());
    }
    line.push(text);
    Ok(())
}

fn sector(file: &[u8], id: u32) -> Result<Vec<u8>> {
    let start = (id as usize)
        .checked_add(1)
        .and_then(|n| n.checked_mul(512))
        .ok_or_else(|| Error::new("xls sector is out of range"))?;
    let end = start + 512;
    if end > file.len() {
        return Err(Error::new("xls sector is truncated"));
    }
    Ok(file[start..end].to_vec())
}

fn read_fat_chain(file: &[u8], fat: &[u8], start: u32, size: Option<u32>) -> Result<Vec<u8>> {
    let want = size.map(|n| n as usize);
    if let Some(0) = want {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut sid = start;
    let mut guard = 0usize;
    while sid != ENDOFCHAIN {
        if sid == FREESECT || sid == FATSECT || sid >= 0xFFFF_FFF0 {
            return Err(Error::new("xls fat chain is refused"));
        }
        guard += 1;
        if guard > fat.len() / 4 + 1 {
            return Err(Error::new("xls fat chain loops"));
        }
        let bytes = sector(file, sid)?;
        if let Some(want) = want {
            let take = (want - out.len()).min(bytes.len());
            out.extend_from_slice(&bytes[..take]);
            if out.len() == want {
                return Ok(out);
            }
        } else {
            out.extend_from_slice(&bytes);
        }
        let at = (sid as usize)
            .checked_mul(4)
            .ok_or_else(|| Error::new("xls fat chain is refused"))?;
        if at + 4 > fat.len() {
            return Err(Error::new("xls fat chain is truncated"));
        }
        sid = u32::from_le_bytes(fat[at..at + 4].try_into().unwrap());
    }
    if want.is_some_and(|want| out.len() != want) {
        return Err(Error::new("xls stream is truncated"));
    }
    Ok(out)
}

fn read_mini_chain(mini_stream: &[u8], minifat: &[u8], start: u32, size: usize) -> Result<Vec<u8>> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(size);
    let mut sid = start;
    let mut guard = 0usize;
    while out.len() < size {
        if sid >= 0xFFFF_FFF0 {
            return Err(Error::new("xls mini fat chain is refused"));
        }
        guard += 1;
        if guard > minifat.len() / 4 + 1 {
            return Err(Error::new("xls mini fat chain loops"));
        }
        let at = (sid as usize) * 64;
        if at + 64 > mini_stream.len() && at >= mini_stream.len() {
            return Err(Error::new("xls mini stream is truncated"));
        }
        let end = (at + 64).min(mini_stream.len());
        let take = (size - out.len()).min(end - at);
        out.extend_from_slice(&mini_stream[at..at + take]);
        if out.len() == size {
            break;
        }
        let fat_at = (sid as usize) * 4;
        if fat_at + 4 > minifat.len() {
            return Err(Error::new("xls mini fat chain is truncated"));
        }
        sid = u32::from_le_bytes(minifat[fat_at..fat_at + 4].try_into().unwrap());
    }
    if out.len() != size {
        return Err(Error::new("xls mini stream is truncated"));
    }
    Ok(out)
}

fn entry_name(entry: &[u8]) -> Result<String> {
    let n = u16::from_le_bytes(entry[0x40..0x42].try_into().unwrap()) as usize;
    if !(2..=64).contains(&n) || n % 2 != 0 {
        return Err(Error::new("xls directory name is refused"));
    }
    let units: Vec<u16> = entry[..n - 2]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).map_err(|_| Error::new("xls directory name is not utf-16"))
}

fn u16_at(file: &[u8], at: usize) -> Result<u16> {
    file.get(at..at + 2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error::new("xls is not BIFF"))
}

fn u32_at(file: &[u8], at: usize) -> Result<u32> {
    file.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error::new("xls is not BIFF"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(stream: &[u8], op: u16) -> &[u8] {
        let mut i = 0;
        while i + 4 <= stream.len() {
            let found = u16::from_le_bytes(stream[i..i + 2].try_into().unwrap());
            let len = u16::from_le_bytes(stream[i + 2..i + 4].try_into().unwrap()) as usize;
            if found == op {
                return &stream[i + 4..i + 4 + len];
            }
            i += 4 + len;
        }
        panic!("missing record {op:#x}");
    }

    #[test]
    fn generated_stream_is_biff8_with_42_and_cat() {
        let stream = workbook_bytes();
        assert_eq!(
            &stream[..8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x05, 0x00]
        );
        let number = record(&stream, 0x0203);
        assert_eq!(number.len(), 14);
        assert_eq!(f64::from_le_bytes(number[6..14].try_into().unwrap()), 42.0);
        let sst = record(&stream, 0x00FC);
        assert_eq!(&sst[8..], &[3, 0, 0, b'c', b'a', b't']);
        let pos = u32::from_le_bytes(record(&stream, 0x0085)[0..4].try_into().unwrap()) as usize;
        assert_eq!(
            &stream[pos..pos + 8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x10, 0x00]
        );
        let file = generated_biff_xls();
        assert_eq!(&file[..8], &OLE_MAGIC);
        assert_eq!(u16::from_le_bytes(file[0x1A..0x1C].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(file[0x1E..0x20].try_into().unwrap()), 9);
    }

    #[test]
    fn two_numbers_keep_the_same_text_cell() {
        let stream = workbook_two_numbers();
        assert_eq!(
            &stream[..8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x05, 0x00]
        );
        let numbers = records(&stream, 0x0203);
        assert_eq!(numbers.len(), 2);
        assert_eq!(u16::from_le_bytes(numbers[0][2..4].try_into().unwrap()), 0);
        assert_eq!(
            f64::from_le_bytes(numbers[0][6..14].try_into().unwrap()),
            42.0
        );
        assert_eq!(u16::from_le_bytes(numbers[1][2..4].try_into().unwrap()), 2);
        assert_eq!(
            f64::from_le_bytes(numbers[1][6..14].try_into().unwrap()),
            7.0
        );
        let text = record(&stream, 0x00FD);
        assert_eq!(u16::from_le_bytes(text[2..4].try_into().unwrap()), 1);
        assert_eq!(&record(&stream, 0x00FC)[8..], &[3, 0, 0, b'c', b'a', b't']);
    }

    #[test]
    fn numeric_column_is_one_thousand_numbers_and_no_formula() {
        let stream = workbook_numeric_column();
        assert!(stream.len() >= 4096);
        assert_eq!(
            &stream[..8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x05, 0x00]
        );
        assert!(records(&stream, 0x0006).is_empty());
        let numbers = records(&stream, 0x0203);
        assert_eq!(numbers.len(), 1000);
        assert_eq!(
            f64::from_le_bytes(numbers[0][6..14].try_into().unwrap()),
            0.0
        );
        assert_eq!(
            f64::from_le_bytes(numbers[999][6..14].try_into().unwrap()),
            999.0
        );
        let file = generated_biff_xls_numeric_column();
        assert_eq!(&file[..8], &OLE_MAGIC);
        assert!(file.len() > stream.len());
    }

    #[test]
    fn numeric_column_two_thousand_reads_every_row() {
        let stream = workbook_numeric_column_two_thousand();
        assert!(stream.len() >= 4096);
        assert!(records(&stream, 0x0006).is_empty());
        let numbers = records(&stream, 0x0203);
        assert_eq!(numbers.len(), 2000);
        assert_eq!(
            f64::from_le_bytes(numbers[0][6..14].try_into().unwrap()),
            0.0
        );
        assert_eq!(
            f64::from_le_bytes(numbers[1999][6..14].try_into().unwrap()),
            1999.0
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(rows.len(), 2000);
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row, &[i.to_string()]);
        }
    }

    #[test]
    fn numeric_column_four_thousand_reads_every_row() {
        assert_eq!(generated_biff_xls_numeric_column().len(), 19_968);
        assert_eq!(
            generated_biff_xls_numeric_column_two_thousand().len(),
            37_888
        );
        let stream = workbook_numeric_column_four_thousand();
        assert!(records(&stream, 0x0006).is_empty());
        let numbers = records(&stream, 0x0203);
        assert_eq!(numbers.len(), 4000);
        assert_eq!(
            f64::from_le_bytes(numbers[0][6..14].try_into().unwrap()),
            0.0
        );
        assert_eq!(
            f64::from_le_bytes(numbers[3999][6..14].try_into().unwrap()),
            3999.0
        );
        let file = generated_biff_xls_numeric_column_four_thousand();
        assert_eq!(u32::from_le_bytes(file[0x2C..0x30].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(file[0x48..0x4C].try_into().unwrap()), 0);
        let rows = read_biff8(&file).unwrap();
        assert_eq!(rows.len(), 4000);
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row, &[i.to_string()]);
        }
    }

    #[test]
    fn sum_formula_returns_the_cached_number_without_reading_the_expression() {
        let stream = workbook_sum_cache();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 36);
        let marker = u16::from_le_bytes(formula[12..14].try_into().unwrap());
        assert_ne!(marker, 0xFFFF);
        assert_eq!(f64::from_le_bytes(formula[6..14].try_into().unwrap()), 5.0);
        assert_eq!(
            &formula[22..],
            &[0x44, 0x00, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x01, 0x00, 0x42, 0x02, 0x04, 0x00,]
        );
        let mut garbled = stream.clone();
        let mut i = 0;
        while i + 4 <= garbled.len() {
            let op = u16::from_le_bytes(garbled[i..i + 2].try_into().unwrap());
            let len = u16::from_le_bytes(garbled[i + 2..i + 4].try_into().unwrap()) as usize;
            if op == 0x0006 {
                for byte in &mut garbled[i + 4 + 22..i + 4 + len] {
                    *byte = 0xFF;
                }
                break;
            }
            i += 4 + len;
        }
        let rows = parse_workbook(&garbled).unwrap();
        assert_eq!(
            rows,
            vec![vec!["2".to_string(), "3".to_string(), "5".to_string()]]
        );
    }

    #[test]
    fn uncached_sum_adds_the_two_numeric_cells() {
        let stream = workbook_sum_uncached();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![vec!["8".to_string(), "9".to_string(), "17".to_string()]]
        );
        let mut other = stream.clone();
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 12] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    #[test]
    fn uncached_sum_adds_three_numeric_cells() {
        let stream = workbook_sum_uncached_three();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 41);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        assert_eq!(
            &formula[22..],
            &[
                0x44, 0x00, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x01, 0x00, 0x44, 0x00, 0x00, 0x02,
                0x00, 0x42, 0x03, 0x04, 0x00,
            ]
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![vec![
                "4".to_string(),
                "5".to_string(),
                "6".to_string(),
                "15".to_string()
            ]]
        );
        let mut other = stream.clone();
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 17] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    #[test]
    fn uncached_sum_adds_four_numeric_cells() {
        let stream = workbook_sum_uncached_four();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 46);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        assert_eq!(
            &formula[22..],
            &[
                0x44, 0x00, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x01, 0x00, 0x44, 0x00, 0x00, 0x02,
                0x00, 0x44, 0x00, 0x00, 0x03, 0x00, 0x42, 0x04, 0x04, 0x00,
            ]
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![vec![
                "4".to_string(),
                "5".to_string(),
                "6".to_string(),
                "7".to_string(),
                "22".to_string()
            ]]
        );
        let mut other = stream.clone();
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 22] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    #[test]
    fn uncached_sum_adds_one_three_cell_range() {
        let stream = workbook_sum_uncached_area();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 35);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        assert_eq!(
            &formula[22..],
            &[0x45, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x42, 0x01, 0x04, 0x00,]
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![vec![
                "1".to_string(),
                "2".to_string(),
                "3".to_string(),
                "6".to_string()
            ]]
        );
        let mut wider = stream.clone();
        let mut i = 0;
        while i + 4 <= wider.len() {
            let op = u16::from_le_bytes(wider[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                wider[i + 4 + 22 + 7] = 4;
                break;
            }
            let len = u16::from_le_bytes(wider[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&wider).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
        let mut other = stream.clone();
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 11] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    #[test]
    fn uncached_sum_adds_one_four_cell_range() {
        let stream = workbook_sum_uncached_area_four();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 35);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        assert_eq!(
            &formula[22..],
            &[0x45, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x42, 0x01, 0x04, 0x00,]
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![vec![
                "1".to_string(),
                "2".to_string(),
                "3".to_string(),
                "4".to_string(),
                "10".to_string()
            ]]
        );
        let mut wider = stream.clone();
        let mut i = 0;
        while i + 4 <= wider.len() {
            let op = u16::from_le_bytes(wider[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                wider[i + 4 + 22 + 7] = 4;
                break;
            }
            let len = u16::from_le_bytes(wider[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&wider).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
        let mut other = stream;
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 11] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    #[test]
    fn uncached_sum_adds_one_three_cell_column() {
        let stream = workbook_sum_uncached_area_column();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 35);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        assert_eq!(
            &formula[22..],
            &[0x45, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x42, 0x01, 0x04, 0x00,]
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![
                vec!["1".to_string()],
                vec!["2".to_string()],
                vec!["3".to_string()],
                vec!["6".to_string()]
            ]
        );
        let mut longer = stream.clone();
        let mut i = 0;
        while i + 4 <= longer.len() {
            let op = u16::from_le_bytes(longer[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                longer[i + 4 + 22 + 3] = 4;
                break;
            }
            let len = u16::from_le_bytes(longer[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&longer).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
        let mut other = stream;
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 11] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    #[test]
    fn uncached_sum_adds_one_four_cell_column() {
        let stream = workbook_sum_uncached_area_column_four();
        let formula = record(&stream, 0x0006);
        assert_eq!(formula.len(), 35);
        assert_eq!(formula[6], 0x03);
        assert_eq!(
            u16::from_le_bytes(formula[12..14].try_into().unwrap()),
            0xFFFF
        );
        assert_eq!(
            &formula[22..],
            &[0x45, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x42, 0x01, 0x04, 0x00,]
        );
        let rows = parse_workbook(&stream).unwrap();
        assert_eq!(
            rows,
            vec![
                vec!["1".to_string()],
                vec!["2".to_string()],
                vec!["3".to_string()],
                vec!["4".to_string()],
                vec!["10".to_string()]
            ]
        );
        let mut longer = stream.clone();
        let mut i = 0;
        while i + 4 <= longer.len() {
            let op = u16::from_le_bytes(longer[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                longer[i + 4 + 22 + 3] = 4;
                break;
            }
            let len = u16::from_le_bytes(longer[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&longer).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
        let mut other = stream;
        let mut i = 0;
        while i + 4 <= other.len() {
            let op = u16::from_le_bytes(other[i..i + 2].try_into().unwrap());
            if op == 0x0006 {
                other[i + 4 + 22 + 11] = 5;
                break;
            }
            let len = u16::from_le_bytes(other[i + 2..i + 4].try_into().unwrap()) as usize;
            i += 4 + len;
        }
        let err = parse_workbook(&other).unwrap_err();
        assert!(err.message().contains("no cached number"), "{err}");
    }

    fn records(stream: &[u8], op: u16) -> Vec<&[u8]> {
        let mut found = Vec::new();
        let mut i = 0;
        while i + 4 <= stream.len() {
            let opcode = u16::from_le_bytes(stream[i..i + 2].try_into().unwrap());
            let len = u16::from_le_bytes(stream[i + 2..i + 4].try_into().unwrap()) as usize;
            if opcode == op {
                found.push(&stream[i + 4..i + 4 + len]);
            }
            i += 4 + len;
        }
        found
    }
}
