//! Table domain inference — a broad content-type hint like "this is a
//! budget table" or "this is an institution-information form". Hints
//! are coarse and conservative on purpose: they help an LLM decide how
//! to chunk or structure-edit a table, but they should never override
//! cell-level role/editable decisions.
//!
//! Heuristic: keyword matching on the table's cell text and on the
//! owning paragraph's text (when the table has a heading nearby).
//! Each keyword contributes one point to its domain; the highest-
//! scoring domain wins when the total reaches [`MIN_SCORE`], otherwise
//! we emit `Unknown` (and callers typically drop the hint entirely).
//!
//! Keywords are the stable Korean-form vocabulary: bureaucratic terms
//! that are unlikely to appear casually in body text. A heading that
//! says "연구비 사용계획" should clearly tag the following table as
//! `Budget` even if the classifier misses a few cells.

use crate::ir::{ControlKind, Paragraph, TableControl};

/// Coarse content-type hint attached to a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableDomain {
    Institution,
    Budget,
    Schedule,
    Performance,
    Personnel,
    Unknown,
}

impl TableDomain {
    /// Snake-case label used in the LLM markdown attribute, e.g.
    /// `kind=budget`.
    pub fn as_str(self) -> &'static str {
        match self {
            TableDomain::Institution => "institution_info",
            TableDomain::Budget => "budget",
            TableDomain::Schedule => "schedule",
            TableDomain::Performance => "performance_metrics",
            TableDomain::Personnel => "personnel",
            TableDomain::Unknown => "unknown",
        }
    }
}

/// Minimum keyword points required before a hint is returned. Below
/// this threshold the table is left as [`TableDomain::Unknown`] and
/// callers typically skip the attribute — better no hint than a wrong
/// one.
pub const MIN_SCORE: u32 = 2;

/// Keyword vocabulary per domain. Each entry is a substring; any
/// occurrence anywhere in the scanned text scores one point. Keywords
/// deliberately avoid single characters — "기관" would match a spurious
/// "기관의 설명" in body prose but the payoff on real forms is worth it.
const KEYWORDS: &[(TableDomain, &[&str])] = &[
    (
        TableDomain::Institution,
        &[
            "기관명",
            "기관 구분",
            "기관구분",
            "대표자",
            "사업자",
            "소재지",
            "사업자등록번호",
            "대표번호",
            "법인등록번호",
            "설립일",
            "주관연구개발기관",
            "위탁연구개발기관",
        ],
    ),
    (
        TableDomain::Budget,
        &[
            "연구비",
            "정부지원",
            "정부 지원",
            "기관 현금",
            "기관 현물",
            "기관현금",
            "기관현물",
            "총 사업비",
            "예산",
            "집행계획",
            "지원금",
            "매칭",
        ],
    ),
    (
        TableDomain::Schedule,
        &[
            "추진일정",
            "추진 일정",
            "연구개발 일정",
            "연차별",
            "분기별",
            "월별",
            "단계별",
            "착수일",
            "종료일",
            "개발 기간",
            "수행 기간",
        ],
    ),
    (
        TableDomain::Performance,
        &[
            "성능지표",
            "성능 지표",
            "최종 개발목표",
            "개발 목표",
            "성과목표",
            "성과 목표",
            "측정 방법",
            "측정방법",
            "달성도",
            "TRL",
            "핵심 지표",
            "핵심지표",
        ],
    ),
    (
        TableDomain::Personnel,
        &[
            "연구책임자",
            "연구 책임자",
            "참여연구원",
            "참여 연구원",
            "연구원 구성",
            "인력 구성",
            "직위",
            "직급",
            "담당 업무",
            "참여율",
        ],
    ),
];

/// Classify `t` using keywords from its own cells *plus* the
/// `owner_para_text` (the paragraph that contains the table — often a
/// heading-in-cell pattern or the outer section title). Text walking
/// is bounded to avoid quadratic cost on deeply-nested tables.
///
/// If keyword matching is inconclusive, falls back to a structural
/// check for gantt-style schedules: tables whose header row is mostly
/// time tokens (months, quarters, 주차, 년차) — those don't carry any
/// Korean-prose keyword the classifier would match on, so they need a
/// shape-based rule.
pub fn infer_table_domain(t: &TableControl, owner_para_text: &str) -> TableDomain {
    let mut corpus = String::new();
    corpus.push_str(owner_para_text);
    corpus.push('\n');
    collect_cell_text(t, &mut corpus, /*budget=*/ 64);

    let keyword_result = score_corpus(&corpus);
    if keyword_result != TableDomain::Unknown {
        return keyword_result;
    }
    if looks_like_schedule(t) {
        return TableDomain::Schedule;
    }
    TableDomain::Unknown
}

/// Structural gantt-schedule detector. A table qualifies when:
///   - it has ≥ 2 rows and ≥ 4 columns (less is too small to be a
///     useful schedule),
///   - its first row has ≥ 3 non-empty cells,
///   - ≥ 70 % of those non-empty cells are "time tokens"
///     ([`is_time_token`]).
///
/// 70 % lets the first cell be a label like "구분" / "활동 내용" without
/// disqualifying the whole table — that left-most cell is the row-label
/// column header and doesn't need to be a time token itself.
fn looks_like_schedule(t: &TableControl) -> bool {
    if t.rows < 2 || t.cols < 4 {
        return false;
    }
    let first_row: Vec<&crate::ir::TableCell> =
        t.cells.iter().filter(|c| c.row == 0).collect();
    let non_empty: Vec<String> = first_row
        .iter()
        .filter_map(|c| {
            let joined: String = c.paragraphs.iter().map(|p| p.text.as_str()).collect();
            let trimmed = joined.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        })
        .collect();
    if non_empty.len() < 3 {
        return false;
    }
    let time_count = non_empty.iter().filter(|s| is_time_token(s)).count();
    time_count * 10 >= non_empty.len() * 7
}

/// Recognise an HWP author's time-column header cell. Accepts:
///   - pure digits, 1–2 characters long (months 1..12, days 1..31,
///     quarters 1..4, etc.),
///   - `NQ` / `Nq` (quarters written in English),
///   - digits followed by a Korean unit: `N월`, `N분기`, `N주차`,
///     `N주`, `N년차`, `N년`.
///
/// Rejects anything else — typical column headers like "구분" or "비고"
/// never match, so a table with those needs real keyword signal to
/// classify as a schedule.
pub(crate) fn is_time_token(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return (1..=2).contains(&s.len());
    }
    for suffix in ["Q", "q", "월", "분기", "주차", "주", "년차", "년"] {
        if let Some(head) = s.strip_suffix(suffix) {
            let head = head.trim();
            if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn score_corpus(text: &str) -> TableDomain {
    let mut best = (TableDomain::Unknown, 0u32);
    for (domain, words) in KEYWORDS {
        let score: u32 = words.iter().map(|w| substr_count(text, w) as u32).sum();
        if score > best.1 {
            best = (*domain, score);
        }
    }
    if best.1 >= MIN_SCORE {
        best.0
    } else {
        TableDomain::Unknown
    }
}

fn substr_count(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

/// Append every cell's text (and recursively, nested-table cells up to
/// `budget` paragraphs total) to `out`. Newline-separated for tokenisers
/// that care about line boundaries.
fn collect_cell_text(t: &TableControl, out: &mut String, mut budget: usize) {
    for cell in &t.cells {
        if budget == 0 {
            return;
        }
        for p in &cell.paragraphs {
            if budget == 0 {
                return;
            }
            budget = budget.saturating_sub(1);
            out.push_str(&p.text);
            out.push('\n');
            for c in &p.controls {
                if let ControlKind::Table(nested) = &c.kind {
                    collect_cell_text(nested, out, budget);
                }
            }
        }
    }
}

/// Small convenience for callers that already have a `Paragraph` and
/// want its cleaned text. Keeps the `domain` module free of
/// export-side dependencies.
pub fn paragraph_text(p: &Paragraph) -> &str {
    p.text.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Paragraph, ParagraphHeader, TableCell};

    fn cell(row: u16, col: u16, text: &str) -> TableCell {
        TableCell {
            row,
            col,
            row_span: 1,
            col_span: 1,
            paragraphs: vec![Paragraph {
                header: ParagraphHeader::default(),
                text: text.into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn table(cells: Vec<TableCell>) -> TableControl {
        TableControl {
            rows: 1,
            cols: cells.len() as u16,
            row_cell_counts: vec![cells.len() as u16],
            cells,
            ..Default::default()
        }
    }

    #[test]
    fn budget_detected_from_header_row_keywords() {
        let t = table(vec![
            cell(0, 0, "구분"),
            cell(0, 1, "정부지원"),
            cell(0, 2, "기관 현금"),
            cell(0, 3, "기관 현물"),
            cell(0, 4, "합계"),
        ]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Budget);
    }

    #[test]
    fn institution_detected_from_label_column() {
        let t = table(vec![
            cell(0, 0, "기관명"),
            cell(0, 1, "(주)벤디트"),
            cell(0, 2, "대표자"),
            cell(0, 3, "홍길동"),
            cell(0, 4, "사업자등록번호"),
        ]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Institution);
    }

    #[test]
    fn schedule_detected_from_timeline_keywords() {
        let t = table(vec![
            cell(0, 0, "추진일정"),
            cell(0, 1, "1분기"),
            cell(0, 2, "2분기"),
            cell(0, 3, "연차별 목표"),
        ]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Schedule);
    }

    #[test]
    fn performance_detected_from_indicator_keywords() {
        let t = table(vec![
            cell(0, 0, "성능지표"),
            cell(0, 1, "측정 방법"),
            cell(0, 2, "최종 개발목표"),
            cell(0, 3, "TRL 7"),
        ]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Performance);
    }

    #[test]
    fn gantt_schedule_detected_without_keywords() {
        // Header row: months 1..12, no prose keywords at all.
        let mut cells: Vec<TableCell> =
            vec![cell(0, 0, "구분")];
        for m in 1..=12 {
            cells.push(cell(0, m, &format!("{m}월")));
        }
        let t = TableControl {
            rows: 2,
            cols: 13,
            row_cell_counts: vec![13, 13],
            cells,
            ..TableControl::default()
        };
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Schedule);
    }

    /// Build a 2-row table: row 0 is `header_cells` (gantt header),
    /// row 1 is filler data cells with the same column count. Needed
    /// because `looks_like_schedule` requires `rows >= 2`.
    fn gantt(header_cells: &[&str]) -> TableControl {
        let cols = header_cells.len() as u16;
        let mut cells: Vec<TableCell> = Vec::new();
        for (c, text) in header_cells.iter().enumerate() {
            cells.push(cell(0, c as u16, text));
        }
        for c in 0..cols {
            cells.push(cell(1, c, ""));
        }
        TableControl {
            rows: 2,
            cols,
            row_cell_counts: vec![cols, cols],
            cells,
            ..TableControl::default()
        }
    }

    #[test]
    fn quarter_schedule_detected() {
        let t = gantt(&["활동 내용", "1Q", "2Q", "3Q", "4Q"]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Schedule);
    }

    #[test]
    fn pure_numeric_header_detected() {
        let t = gantt(&["구분", "1", "2", "3", "4"]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Schedule);
    }

    #[test]
    fn short_table_not_a_schedule() {
        // < 4 cols — structural check bails.
        let t = gantt(&["구분", "1월", "2월"]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Unknown);
    }

    #[test]
    fn non_time_header_not_a_schedule() {
        let t = gantt(&["항목", "내용", "비고", "단위", "가격"]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Unknown);
    }

    #[test]
    fn keyword_match_still_wins_over_structural() {
        // Numeric header *and* a strong budget keyword in the heading.
        // Keyword branch should take precedence.
        let t = gantt(&["구분", "1", "2", "3", "4"]);
        assert_eq!(
            infer_table_domain(&t, "6. 연구비 사용계획 (예산 집행)"),
            TableDomain::Budget
        );
    }

    #[test]
    fn is_time_token_recognises_common_forms() {
        assert!(is_time_token("1"));
        assert!(is_time_token("12"));
        assert!(is_time_token("1월"));
        assert!(is_time_token("4분기"));
        assert!(is_time_token("1Q"));
        assert!(is_time_token("2년차"));
        assert!(is_time_token("3주차"));
        // Not time tokens:
        assert!(!is_time_token("구분"));
        assert!(!is_time_token("내용"));
        assert!(!is_time_token("100"));
        assert!(!is_time_token(""));
        assert!(!is_time_token("1월 실적"));
    }

    #[test]
    fn unrelated_table_stays_unknown() {
        // No domain keywords at all.
        let t = table(vec![
            cell(0, 0, "항목"),
            cell(0, 1, "내용"),
            cell(0, 2, "비고"),
        ]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Unknown);
    }

    #[test]
    fn owner_heading_adds_signal() {
        // Cells are vague, but the owning paragraph text strongly
        // suggests budget.
        let t = table(vec![
            cell(0, 0, "A"),
            cell(0, 1, "B"),
            cell(0, 2, "C"),
        ]);
        // Heading contributes two Budget keywords: "예산" + "연구비".
        assert_eq!(
            infer_table_domain(&t, "6. 연구비 사용계획 (예산 집행)"),
            TableDomain::Budget
        );
    }

    #[test]
    fn single_keyword_hit_stays_unknown() {
        // One match doesn't reach MIN_SCORE (=2). Prefer no hint to a
        // shaky hint.
        let t = table(vec![
            cell(0, 0, "항목"),
            cell(0, 1, "TRL"), // one performance keyword
            cell(0, 2, "비고"),
        ]);
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Unknown);
    }

    #[test]
    fn ties_break_by_iteration_order() {
        // When scores tie, the first domain in KEYWORDS order wins.
        // Institution appears before Budget, so Institution takes two
        // vs Budget two.
        let t = table(vec![
            cell(0, 0, "기관명"),      // institution
            cell(0, 1, "대표자"),       // institution
            cell(0, 2, "예산"),         // budget
            cell(0, 3, "연구비"),       // budget
        ]);
        // With strict > (not >=), Institution wins because it's listed
        // first and takes the lead first.
        assert_eq!(infer_table_domain(&t, ""), TableDomain::Institution);
    }
}
