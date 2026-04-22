//! Per-stream typed encoders. Each module owns one OLE stream path;
//! exactly one of (`STREAM_NAME`, `parse()`, `emit()`) is visible per stream.
//! Streams not covered here travel through `IrDocument::unknown_streams`.

pub mod bin_data;
pub mod body_text;
pub mod border_fill;
pub mod char_shape;
pub mod ctrl_header;
pub mod doc_info;
pub mod document_properties;
pub mod face_name;
pub mod file_header;
pub mod gso_picture;
pub mod id_mappings;
pub mod list_header;
pub mod para_shape;
pub mod paragraph_char_shape;
pub mod paragraph_header;
pub mod paragraph_line_seg;
pub mod paragraph_text;
pub mod record;
pub mod style;
pub mod table;
