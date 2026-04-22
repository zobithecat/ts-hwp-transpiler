pub mod domain;
pub mod grid;
pub mod visual;
pub mod visual_adapter;

pub use domain::{TableDomain, infer_table_domain};
pub use grid::{GridCoord, GridSlot, TableGrid};
pub use visual::{
    BgTone, BorderFillResolver, CellCoord, CellRole, CellVisualFingerprint,
    Hue, Rgba, VisualExtract, classify_roles,
};
pub use visual_adapter::DocInfoResolver;
