//! Visual fingerprint extraction for HWP table cells.
//!
//! HWP stores a cell's background under `border_fill_id → BorderFill.fill`,
//! *not* on the cell itself, so the extractor needs a resolver. The output
//! `CellVisualFingerprint` is the joint (bg + coord + border) vector that the
//! downstream Label/Header/Content classifier consumes.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self { r: 0, g: 0, b: 0, a: 0 };
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };

    /// Rec. 709 perceived luminance (0..=255). Primary axis for the
    /// "colored label vs white content" heuristic in Korean report tables.
    pub fn luminance(self) -> u8 {
        let y = 0.2126 * self.r as f32
            + 0.7152 * self.g as f32
            + 0.0722 * self.b as f32;
        y.min(255.0) as u8
    }

    pub fn is_transparent(self) -> bool {
        self.a == 0
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Coarse hue+brightness bucket. Keeps the fingerprint stable under minor
/// color drift (HWP authors frequently nudge `#FFFF99` ↔ `#FFFFAA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum BgTone {
    None,
    Pale,
    Mid,
    Dark,
    Accent(Hue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Hue {
    Red,
    Yellow,
    Green,
    Cyan,
    Blue,
    Magenta,
}

/// Physical coords identical to `hwp::Cell { column, row, col_span, row_span }`.
/// Logical (i, j) indexing lives in [`super::grid::TableGrid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct CellCoord {
    pub col: u16,
    pub row: u16,
    pub col_span: u16,
    pub row_span: u16,
}

/// Full visual signature of one cell. `Hash + Eq` so a whole table can be
/// clustered into visual classes (e.g. "all yellow-tinted label cells").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CellVisualFingerprint {
    pub coord: CellCoord,
    pub bg: BgTone,
    pub bg_rgba: Rgba,
    /// `[top, right, bottom, left]` — matches HWP BorderFill::borders order.
    pub borders: [bool; 4],
    pub is_first_row: bool,
    pub is_first_col: bool,
}

impl CellVisualFingerprint {
    /// Stable key for clustering. Drops exact RGBA but keeps the tone bucket,
    /// so `#FFFF99` and `#FFFF88` collapse to the same label class.
    pub fn class_key(&self) -> (BgTone, [bool; 4], bool, bool) {
        (self.bg, self.borders, self.is_first_row, self.is_first_col)
    }
}

/// The adapter must implement this to bridge cell → DocInfo. Keeping it as a
/// trait (not a concrete table lookup) lets tests inject fake resolvers.
pub trait BorderFillResolver {
    fn resolve_bg(&self, border_fill_id: u16) -> Option<Rgba>;
    /// `[top, right, bottom, left]`
    fn resolve_borders(&self, border_fill_id: u16) -> [bool; 4];
}

/// Implemented on the foreign cell type (e.g. `hwp::Cell`). The core crate
/// never references the parser's concrete types directly.
pub trait VisualExtract {
    fn coord(&self) -> CellCoord;
    fn border_fill_id(&self) -> u16;

    fn fingerprint<R: BorderFillResolver>(
        &self,
        resolver: &R,
        is_first_row: bool,
        is_first_col: bool,
    ) -> CellVisualFingerprint {
        let id = self.border_fill_id();
        let bg_rgba = resolver.resolve_bg(id).unwrap_or(Rgba::TRANSPARENT);
        CellVisualFingerprint {
            coord: self.coord(),
            bg: BgTone::classify(bg_rgba),
            bg_rgba,
            borders: resolver.resolve_borders(id),
            is_first_row,
            is_first_col,
        }
    }
}

impl BgTone {
    pub fn classify(c: Rgba) -> Self {
        if c.is_transparent() {
            return BgTone::None;
        }
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        let chroma = max - min;
        let l = c.luminance();

        // Near-gray: bucket by luminance.
        if chroma < 12 {
            return match l {
                0..=119 => BgTone::Dark,
                120..=199 => BgTone::Mid,
                200..=240 => BgTone::Pale,
                _ => BgTone::None,
            };
        }

        // Chromatic: crude 6-hue bucket. Enough for label detection — exact
        // color is still available in `bg_rgba` for rendering.
        let (r, g, b) = (c.r, c.g, c.b);
        let hue = if r >= g && r >= b {
            if b >= g {
                Hue::Magenta
            } else if g as i16 >= r as i16 - 40 {
                Hue::Yellow
            } else {
                Hue::Red
            }
        } else if g >= r && g >= b {
            if b >= r {
                Hue::Cyan
            } else {
                Hue::Green
            }
        } else if r >= g {
            Hue::Magenta
        } else {
            Hue::Blue
        };
        BgTone::Accent(hue)
    }
}

/// Semantic role derived from the full-table context. Separate from
/// fingerprinting because the classifier needs to see every cell to judge
/// relative darkness / coverage (e.g. "is *this* the darkest row").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CellRole {
    Header,
    Label,
    Content,
    Spacer,
}

/// Baseline heuristic labeler. Tuned for Korean government-report tables
/// (yellow/mid-gray label column + dark header row).
pub fn classify_roles(cells: &[CellVisualFingerprint]) -> Vec<CellRole> {
    let has_label_tone = cells.iter().any(|c| {
        matches!(
            c.bg,
            BgTone::Accent(Hue::Yellow) | BgTone::Mid | BgTone::Dark
        )
    });

    cells
        .iter()
        .map(|c| match (c.bg, c.is_first_row, c.is_first_col) {
            (BgTone::Dark, _, _) => CellRole::Header,
            (BgTone::Accent(Hue::Yellow), _, _) => CellRole::Label,
            (BgTone::Mid, _, _) if has_label_tone => CellRole::Label,
            (BgTone::Pale, true, _) => CellRole::Header,
            (_, true, false) => CellRole::Header,
            (_, _, true) if has_label_tone => CellRole::Label,
            (BgTone::None, _, _) => CellRole::Content,
            _ => CellRole::Content,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yellow_cell_is_labeled() {
        let yellow = Rgba { r: 0xFF, g: 0xFF, b: 0x99, a: 0xFF };
        assert!(matches!(BgTone::classify(yellow), BgTone::Accent(Hue::Yellow)));
    }

    #[test]
    fn dark_header_wins_over_first_row() {
        let fp = CellVisualFingerprint {
            coord: CellCoord { col: 0, row: 0, col_span: 1, row_span: 1 },
            bg: BgTone::Dark,
            bg_rgba: Rgba { r: 30, g: 30, b: 30, a: 255 },
            borders: [true; 4],
            is_first_row: true,
            is_first_col: true,
        };
        assert_eq!(classify_roles(&[fp])[0], CellRole::Header);
    }
}
