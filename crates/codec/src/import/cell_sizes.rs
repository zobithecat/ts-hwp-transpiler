//! Default cell sizing for from-scratch markdown tables.
//!
//! HWPX's `<hp:cellSz width=… height=…/>` is an absolute geometry
//! statement — viewers don't auto-size cells from text content the
//! way GFM tables do in HTML. A cell whose `width_hwpu` /
//! `height_hwpu` is `0` collapses to nothing, so an MD-imported
//! table renders as an empty stripe.
//!
//! `apply_defaults` walks freshly-built [`TableCell`]s, gives them
//! sensible per-column widths and per-row heights so the table
//! occupies the standard A4 text region, and fills in
//! `text_width_hwpu` (which the HWP5 writer also expects). Only
//! cells whose values are still zero get touched, so a future
//! parser path that surfaces explicit cell widths from the source
//! Markdown (e.g. via attribute extensions) can keep them.

use hwp_transpiler_core::ir::TableCell;

/// A4 portrait text region width in HWPUNIT (1 mm = 283 HWPUNIT,
/// roughly). 150 mm covers `210 - 30 - 30` page minus left/right
/// margins, the most common default in Hancom-authored docs.
const A4_TEXT_WIDTH_HWPU: u32 = 42_000;

/// Single text-row height. Empirically ~1500 HWPUNIT for body text;
/// matches what we see in TRL fixtures' default cells.
const DEFAULT_ROW_HEIGHT_HWPU: u32 = 1_500;

/// Conservative left+right cell padding total. Subtracted from
/// `width_hwpu` to derive `text_width_hwpu`.
const DEFAULT_HORIZONTAL_PADDING_HWPU: u32 = 200;

pub fn apply_defaults(cells: &mut [TableCell], cols: u16) {
    if cols == 0 {
        return;
    }
    let per_col = A4_TEXT_WIDTH_HWPU / cols as u32;
    for cell in cells.iter_mut() {
        if cell.width_hwpu == 0 {
            cell.width_hwpu = per_col * cell.col_span.max(1) as u32;
        }
        if cell.height_hwpu == 0 {
            cell.height_hwpu = DEFAULT_ROW_HEIGHT_HWPU * cell.row_span.max(1) as u32;
        }
        if cell.text_width_hwpu == 0 {
            cell.text_width_hwpu = cell
                .width_hwpu
                .saturating_sub(DEFAULT_HORIZONTAL_PADDING_HWPU);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(row: u16, col: u16, row_span: u16, col_span: u16) -> TableCell {
        TableCell {
            row,
            col,
            row_span,
            col_span,
            ..TableCell::default()
        }
    }

    #[test]
    fn unit_cells_get_per_column_width() {
        let mut cells = vec![cell(0, 0, 1, 1), cell(0, 1, 1, 1)];
        apply_defaults(&mut cells, 2);
        assert_eq!(cells[0].width_hwpu, A4_TEXT_WIDTH_HWPU / 2);
        assert_eq!(cells[1].width_hwpu, A4_TEXT_WIDTH_HWPU / 2);
        assert!(cells[0].height_hwpu > 0);
        assert!(cells[0].text_width_hwpu > 0 && cells[0].text_width_hwpu < cells[0].width_hwpu);
    }

    #[test]
    fn col_span_proportional_to_span() {
        let mut cells = vec![cell(0, 0, 1, 3), cell(0, 3, 1, 1)];
        apply_defaults(&mut cells, 4);
        let unit = A4_TEXT_WIDTH_HWPU / 4;
        assert_eq!(cells[0].width_hwpu, unit * 3);
        assert_eq!(cells[1].width_hwpu, unit);
    }

    #[test]
    fn row_span_extends_height() {
        let mut cells = vec![cell(0, 0, 2, 1)];
        apply_defaults(&mut cells, 1);
        assert_eq!(cells[0].height_hwpu, DEFAULT_ROW_HEIGHT_HWPU * 2);
    }

    #[test]
    fn pre_existing_widths_preserved() {
        let mut cells = vec![{
            let mut c = cell(0, 0, 1, 1);
            c.width_hwpu = 9999;
            c.height_hwpu = 7777;
            c.text_width_hwpu = 1234;
            c
        }];
        apply_defaults(&mut cells, 1);
        assert_eq!(cells[0].width_hwpu, 9999);
        assert_eq!(cells[0].height_hwpu, 7777);
        assert_eq!(cells[0].text_width_hwpu, 1234);
    }

    #[test]
    fn zero_cols_no_panic() {
        let mut cells = vec![cell(0, 0, 1, 1)];
        apply_defaults(&mut cells, 0);
        assert_eq!(cells[0].width_hwpu, 0);
    }
}
