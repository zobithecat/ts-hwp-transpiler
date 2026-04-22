//! Adapters that plug HWP5 IR types into the `semantics::visual`
//! classifier. Kept in a separate module so the trait definitions in
//! `visual.rs` stay parser-agnostic — this file is where the
//! `DocInfo` / `TableCell` lookup glue actually lives.

use crate::ir::{DocInfo, TableCell};

use super::visual::{BorderFillResolver, CellCoord, Rgba, VisualExtract};

/// `BorderFillResolver` backed by `DocInfo.border_fills`. HWP5 uses
/// 1-based ids (hwplib convention) — id `0` is a reserved "no style"
/// marker and returns `None`. `id-1` is the slice index.
pub struct DocInfoResolver<'a> {
    pub doc_info: &'a DocInfo,
}

impl<'a> DocInfoResolver<'a> {
    pub fn new(doc_info: &'a DocInfo) -> Self {
        Self { doc_info }
    }
}

impl<'a> BorderFillResolver for DocInfoResolver<'a> {
    fn resolve_bg(&self, border_fill_id: u16) -> Option<Rgba> {
        if border_fill_id == 0 {
            return None;
        }
        let bf = self.doc_info.border_fills.get((border_fill_id as usize) - 1)?;
        let (r, g, b, a) = bf.fill.back_color()?;
        Some(Rgba { r, g, b, a })
    }

    fn resolve_borders(&self, border_fill_id: u16) -> [bool; 4] {
        if border_fill_id == 0 {
            return [false; 4];
        }
        let Some(bf) = self.doc_info.border_fills.get((border_fill_id as usize) - 1)
        else {
            return [false; 4];
        };
        // BorderFill.borders is ordered [left, right, top, bottom] per
        // the IR type; the classifier expects [top, right, bottom, left].
        // `Border::kind` == 0 conventionally means "no border".
        let [left, right, top, bottom] = &bf.borders;
        [top.kind != 0, right.kind != 0, bottom.kind != 0, left.kind != 0]
    }
}

impl VisualExtract for TableCell {
    fn coord(&self) -> CellCoord {
        CellCoord {
            col: self.col,
            row: self.row,
            col_span: self.col_span,
            row_span: self.row_span,
        }
    }
    fn border_fill_id(&self) -> u16 {
        self.border_fill_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BorderFill, Border, Fill};

    #[test]
    fn resolver_reads_color_at_id_minus_one() {
        let mut doc_info = DocInfo::default();
        // id=1 → yellow fill (first entry)
        doc_info.border_fills.push(BorderFill {
            fill: Fill {
                kind: Fill::KIND_COLOR,
                body: vec![0xFF, 0xFF, 0x99, 0xFF, 0, 0, 0, 0],
            },
            ..BorderFill::default()
        });
        let r = DocInfoResolver::new(&doc_info);
        assert_eq!(
            r.resolve_bg(1),
            Some(Rgba { r: 0xFF, g: 0xFF, b: 0x99, a: 0xFF })
        );
    }

    #[test]
    fn resolver_zero_id_is_none() {
        let doc_info = DocInfo::default();
        let r = DocInfoResolver::new(&doc_info);
        assert!(r.resolve_bg(0).is_none());
    }

    #[test]
    fn resolver_out_of_range_is_none() {
        let doc_info = DocInfo::default();
        let r = DocInfoResolver::new(&doc_info);
        assert!(r.resolve_bg(99).is_none());
    }

    #[test]
    fn resolver_maps_border_order_to_classifier_expectations() {
        let mut doc_info = DocInfo::default();
        // left.kind=1, right.kind=0, top.kind=2, bottom.kind=0
        doc_info.border_fills.push(BorderFill {
            borders: [
                Border { kind: 1, ..Border::default() }, // left
                Border { kind: 0, ..Border::default() }, // right
                Border { kind: 2, ..Border::default() }, // top
                Border { kind: 0, ..Border::default() }, // bottom
            ],
            ..BorderFill::default()
        });
        let r = DocInfoResolver::new(&doc_info);
        // classifier expects [top, right, bottom, left]
        assert_eq!(r.resolve_borders(1), [true, false, false, true]);
    }

    #[test]
    fn non_color_fill_returns_none_bg() {
        let mut doc_info = DocInfo::default();
        doc_info.border_fills.push(BorderFill {
            fill: Fill {
                kind: Fill::KIND_IMAGE,
                body: vec![0xFF, 0xFF, 0x99, 0xFF],
            },
            ..BorderFill::default()
        });
        let r = DocInfoResolver::new(&doc_info);
        assert!(r.resolve_bg(1).is_none());
    }
}
