//! Thin wrapper around the `cfb` crate. Isolates the rest of the writer
//! from OLE Compound Document specifics.

use std::io::{self, Cursor};

pub type InMemoryCompound = cfb::CompoundFile<Cursor<Vec<u8>>>;

/// Create an empty in-memory compound file. Returned value can be handed to
/// stream encoders which call `create_stream` / `open_stream` on it.
pub fn new_in_memory() -> io::Result<InMemoryCompound> {
    cfb::CompoundFile::create(Cursor::new(Vec::new()))
}

/// Flush the compound file and return the raw bytes. Call once, at the end
/// of the pipeline.
pub fn finalize(mut comp: InMemoryCompound) -> io::Result<Vec<u8>> {
    comp.flush()?;
    Ok(comp.into_inner().into_inner())
}
