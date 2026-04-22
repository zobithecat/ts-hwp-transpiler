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

/// Heuristic labeler. Tuned for Korean government-report tables, which
/// use a broader palette than just yellow labels + dark headers:
///
/// - Light-blue / light-cyan / light-magenta tints (`#DFE6F7`, `#E8F0E8`,
///   etc.) as label-column backgrounds.
/// - Pale gray (`#E5E5E5`) as label-column backgrounds too, when the
///   table has an obvious header/label pattern elsewhere.
/// - Yellow (`#FFFF99`) as classic label accent (kept).
/// - Dark (any luminance < 120) as header banner (kept).
///
/// Red/warm reds are *excluded* from label-tone detection because HWP
/// authors use those for emphasis ("주의", "필수") inside values rather
/// than for labels.
pub fn classify_roles(cells: &[CellVisualFingerprint]) -> Vec<CellRole> {
    let has_label_tone = cells.iter().any(|c| {
        is_label_tone(c) || matches!(c.bg, BgTone::Mid | BgTone::Dark)
    });

    cells
        .iter()
        .map(|c| match (c.bg, c.is_first_row, c.is_first_col) {
            (BgTone::Dark, _, _) => CellRole::Header,
            // Any label-coded tone (yellow, pale non-red accent, etc.)
            // is a label regardless of position.
            _ if is_label_tone(c) => CellRole::Label,
            (BgTone::Mid, _, _) if has_label_tone => CellRole::Label,
            // Pale first-column cells in a table that already has a
            // label pattern — treat the pale gray column as label.
            (BgTone::Pale, _, true) if has_label_tone => CellRole::Label,
            (BgTone::Pale, true, _) => CellRole::Header,
            (_, true, false) => CellRole::Header,
            (_, _, true) if has_label_tone => CellRole::Label,
            (BgTone::None, _, _) => CellRole::Content,
            _ => CellRole::Content,
        })
        .collect()
}

/// Is this cell's background colour a "label tone" — an author's
/// deliberate tint to distinguish label/key cells from content cells?
///
///   - Yellow accent at any luminance (classic label, always intended).
///   - Any non-red accent with luminance ≥ 200 (pale blue, pale green,
///     pale magenta, pale cyan): dominant Korean-form idiom for label
///     column tints like `#DFE6F7`.
///
/// Red is excluded because HWP forms use red/pink for emphasis inside
/// *values* (missing-required marks,警告) rather than for labels.
fn is_label_tone(fp: &CellVisualFingerprint) -> bool {
    match fp.bg {
        BgTone::Accent(Hue::Yellow) => true,
        BgTone::Accent(Hue::Red) => false,
        BgTone::Accent(_) => fp.bg_rgba.luminance() >= 200,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(bg: BgTone, rgba: Rgba, first_row: bool, first_col: bool) -> CellVisualFingerprint {
        CellVisualFingerprint {
            coord: CellCoord { col: 0, row: 0, col_span: 1, row_span: 1 },
            bg,
            bg_rgba: rgba,
            borders: [true; 4],
            is_first_row: first_row,
            is_first_col: first_col,
        }
    }

    #[test]
    fn yellow_cell_is_labeled() {
        let yellow = Rgba { r: 0xFF, g: 0xFF, b: 0x99, a: 0xFF };
        assert!(matches!(BgTone::classify(yellow), BgTone::Accent(Hue::Yellow)));
    }

    #[test]
    fn dark_header_wins_over_first_row() {
        assert_eq!(
            classify_roles(&[fp(
                BgTone::Dark,
                Rgba { r: 30, g: 30, b: 30, a: 255 },
                true,
                true,
            )])[0],
            CellRole::Header
        );
    }

    #[test]
    fn pale_blue_accent_is_labeled() {
        // #DFE6F7 — Korean-form light-blue label column tone. Classifier
        // must recognise it as Label even without Yellow.
        let blue = Rgba { r: 0xDF, g: 0xE6, b: 0xF7, a: 0xFF };
        let tone = BgTone::classify(blue);
        assert!(matches!(tone, BgTone::Accent(_)), "expected Accent, got {tone:?}");
        assert!(blue.luminance() >= 200);

        let roles = classify_roles(&[fp(tone, blue, false, true)]);
        assert_eq!(roles[0], CellRole::Label);
    }

    #[test]
    fn pale_green_accent_is_labeled() {
        let green = Rgba { r: 0xE0, g: 0xF0, b: 0xE0, a: 0xFF };
        let tone = BgTone::classify(green);
        assert_eq!(
            classify_roles(&[fp(tone, green, false, false)])[0],
            CellRole::Label
        );
    }

    #[test]
    fn pale_red_accent_is_not_labeled() {
        // Red tones signal emphasis / warnings in HWP forms, never
        // label columns. Use #FFCC99 — a pale warm tone the classifier
        // tags as Hue::Red (b=153 < g=204, so it bypasses the Magenta
        // branch).
        let red = Rgba { r: 0xFF, g: 0xCC, b: 0x99, a: 0xFF };
        let tone = BgTone::classify(red);
        assert!(
            matches!(tone, BgTone::Accent(Hue::Red)),
            "fixture must classify as Red; got {tone:?}"
        );
        assert!(red.luminance() >= 200);
        let roles = classify_roles(&[fp(tone, red, false, false)]);
        assert_ne!(roles[0], CellRole::Label, "Red must not be label");
    }

    #[test]
    fn mid_luminance_accent_is_not_labeled() {
        // Mid-luminance saturated blue — too dark to be a label tint,
        // typically signals a button/header rather than a label column.
        let mid_blue = Rgba { r: 0x40, g: 0x70, b: 0xB0, a: 0xFF };
        let tone = BgTone::classify(mid_blue);
        let roles = classify_roles(&[fp(tone, mid_blue, false, false)]);
        assert_ne!(roles[0], CellRole::Label);
    }

    #[test]
    fn pale_gray_first_col_with_label_pattern_is_labeled() {
        // #E5E5E5 first-column cell, table also has a yellow label
        // somewhere → gray column reads as label too.
        let gray = Rgba { r: 0xE5, g: 0xE5, b: 0xE5, a: 0xFF };
        let yellow = Rgba { r: 0xFF, g: 0xFF, b: 0x99, a: 0xFF };
        let cells = vec![
            fp(BgTone::classify(yellow), yellow, true, true),
            fp(BgTone::classify(gray), gray, false, true),
        ];
        let roles = classify_roles(&cells);
        assert_eq!(roles[0], CellRole::Label, "yellow still label");
        assert_eq!(roles[1], CellRole::Label, "pale gray first-col inherits label");
    }
}
