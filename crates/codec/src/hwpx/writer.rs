use hwp_transpiler_core::ir::{IrDocument, IrError, Writer};

#[derive(Default)]
pub struct HwpxWriter;

impl Writer for HwpxWriter {
    fn write(&mut self, _doc: &IrDocument) -> Result<Vec<u8>, IrError> {
        Err(IrError::Unsupported("HwpxWriter not yet implemented".into()))
    }
}
