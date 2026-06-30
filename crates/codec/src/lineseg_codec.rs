//! Compact textual codec for a paragraph's `PARA_LINE_SEG` layout,
//! used to carry line geometry across the LLM-Markdown round-trip.
//!
//! Line segments are layout cache, not content — but a multi-line
//! paragraph that loses them re-emits as a single seed `<hp:lineseg>`,
//! which makes cache-trusting HWPX viewers stack every wrapped line
//! (and the next paragraph) at the same Y. So the geometry is frozen
//! on the `lineseg=` attribute of the PARAGRAPH / TEXT record it
//! belongs to — travelling *with* the paragraph, so no separate
//! id-keying is needed and reordering edits can't mismap it.
//!
//! Format: segments joined by `|`, each segment nine `:`-separated
//! ints in `LineSegment` field order:
//! `text_start:vertical_position:line_height:text_height:baseline_distance:line_spacing:start_x:width:tag`

use hwp_transpiler_core::ir::LineSegment;

/// Serialise line segments to the compact `lineseg=` value. Empty
/// input yields an empty string (caller should then omit the attr).
pub fn encode(segs: &[LineSegment]) -> String {
    segs.iter()
        .map(|s| {
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}",
                s.text_start,
                s.vertical_position_hwpu,
                s.line_height_hwpu,
                s.text_height_hwpu,
                s.baseline_distance_hwpu,
                s.line_spacing_hwpu,
                s.start_x_hwpu,
                s.width_hwpu,
                s.tag,
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// Parse a `lineseg=` value back into segments. Any malformed segment
/// (wrong field count or non-numeric field) is skipped rather than
/// failing the whole import — a partial layout still beats none.
pub fn decode(value: &str) -> Vec<LineSegment> {
    value
        .split('|')
        .filter(|s| !s.is_empty())
        .filter_map(|seg| {
            let f: Vec<&str> = seg.split(':').collect();
            if f.len() != 9 {
                return None;
            }
            Some(LineSegment {
                text_start: f[0].parse().ok()?,
                vertical_position_hwpu: f[1].parse().ok()?,
                line_height_hwpu: f[2].parse().ok()?,
                text_height_hwpu: f[3].parse().ok()?,
                baseline_distance_hwpu: f[4].parse().ok()?,
                line_spacing_hwpu: f[5].parse().ok()?,
                start_x_hwpu: f[6].parse().ok()?,
                width_hwpu: f[7].parse().ok()?,
                tag: f[8].parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let segs = vec![
            LineSegment {
                text_start: 0,
                vertical_position_hwpu: 0,
                line_height_hwpu: 1320,
                text_height_hwpu: 1000,
                baseline_distance_hwpu: 850,
                line_spacing_hwpu: 600,
                start_x_hwpu: 0,
                width_hwpu: 42520,
                tag: 393216,
            },
            LineSegment {
                text_start: 25,
                vertical_position_hwpu: 1320,
                line_height_hwpu: 1320,
                text_height_hwpu: 1000,
                baseline_distance_hwpu: 850,
                line_spacing_hwpu: 600,
                start_x_hwpu: 0,
                width_hwpu: 42520,
                tag: 1,
            },
        ];
        assert_eq!(decode(&encode(&segs)), segs);
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(encode(&[]), "");
        assert_eq!(decode(""), Vec::<LineSegment>::new());
    }

    #[test]
    fn malformed_segment_skipped() {
        // second segment has 8 fields, not 9 — dropped, first kept.
        let v = "0:0:1320:1000:850:600:0:42520:393216|bad:1:2";
        assert_eq!(decode(v).len(), 1);
    }
}
