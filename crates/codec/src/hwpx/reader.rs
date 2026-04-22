use hwp_transpiler_core::ir::{IrDocument, IrError, Reader};

#[derive(Default)]
pub struct HwpxReader;

impl Reader for HwpxReader {
    fn read(&mut self, _input: &[u8]) -> Result<IrDocument, IrError> {
        Err(IrError::Unsupported("HwpxReader not yet implemented".into()))
    }
}
