pub mod grid;
pub mod visual;

pub use grid::{GridCoord, GridSlot, TableGrid};
pub use visual::{
    BgTone, BorderFillResolver, CellCoord, CellRole, CellVisualFingerprint,
    Hue, Rgba, VisualExtract, classify_roles,
};
