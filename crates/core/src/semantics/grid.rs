//! $N \times M$ logical grid. Translates physical `(col, row)` + spans into a
//! dense matrix where each slot is either `Owner`, `CoveredBy(owner_coord)`,
//! or `Empty` (0x0 / broken span).

use crate::semantics::visual::CellVisualFingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridSlot {
    Owner(u32),
    CoveredBy { col: u16, row: u16 },
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridCoord {
    pub i: u16,
    pub j: u16,
}

pub struct TableGrid {
    pub rows: u16,
    pub cols: u16,
    slots: Vec<GridSlot>,
    cells: Vec<CellVisualFingerprint>,
}

impl TableGrid {
    pub fn new(rows: u16, cols: u16) -> Self {
        let n = rows as usize * cols as usize;
        Self {
            rows,
            cols,
            slots: vec![GridSlot::Empty; n],
            cells: Vec::new(),
        }
    }

    /// Registers a cell at its physical coord and stamps every covered slot.
    /// Out-of-range spans are silently clipped — HWP files in the wild do
    /// ship broken span values.
    pub fn place(&mut self, fp: CellVisualFingerprint) {
        let idx = self.cells.len() as u32;
        let c = fp.coord;
        let (row_span, col_span) = (c.row_span.max(1), c.col_span.max(1));

        for dr in 0..row_span {
            for dc in 0..col_span {
                let (r, k) = (c.row + dr, c.col + dc);
                if r >= self.rows || k >= self.cols {
                    continue;
                }
                let slot_idx = r as usize * self.cols as usize + k as usize;
                self.slots[slot_idx] = if dr == 0 && dc == 0 {
                    GridSlot::Owner(idx)
                } else {
                    GridSlot::CoveredBy { col: c.col, row: c.row }
                };
            }
        }
        self.cells.push(fp);
    }

    pub fn at(&self, coord: GridCoord) -> GridSlot {
        self.slots[coord.i as usize * self.cols as usize + coord.j as usize]
    }

    /// Resolves covered slots back to their owner. Returns `None` for empty
    /// positions.
    pub fn owner(&self, coord: GridCoord) -> Option<&CellVisualFingerprint> {
        match self.at(coord) {
            GridSlot::Owner(i) => self.cells.get(i as usize),
            GridSlot::CoveredBy { col, row } => {
                self.owner(GridCoord { i: row, j: col })
            }
            GridSlot::Empty => None,
        }
    }

    pub fn cells(&self) -> &[CellVisualFingerprint] {
        &self.cells
    }

    pub fn row(&self, i: u16) -> impl Iterator<Item = (GridCoord, GridSlot)> + '_ {
        (0..self.cols).map(move |j| {
            let gc = GridCoord { i, j };
            (gc, self.at(gc))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::visual::{BgTone, CellCoord, Rgba};

    fn fp(col: u16, row: u16, cs: u16, rs: u16) -> CellVisualFingerprint {
        CellVisualFingerprint {
            coord: CellCoord { col, row, col_span: cs, row_span: rs },
            bg: BgTone::None,
            bg_rgba: Rgba::TRANSPARENT,
            borders: [false; 4],
            is_first_row: row == 0,
            is_first_col: col == 0,
        }
    }

    #[test]
    fn merged_cell_covers_neighbours() {
        let mut g = TableGrid::new(2, 3);
        g.place(fp(0, 0, 2, 1));
        g.place(fp(2, 0, 1, 2));
        g.place(fp(0, 1, 1, 1));
        g.place(fp(1, 1, 1, 1));

        assert!(matches!(
            g.at(GridCoord { i: 0, j: 1 }),
            GridSlot::CoveredBy { col: 0, row: 0 }
        ));
        let owner = g.owner(GridCoord { i: 0, j: 1 }).unwrap();
        assert_eq!(owner.coord.col, 0);
        assert_eq!(owner.coord.row, 0);
    }
}
