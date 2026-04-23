//! HWP equation script → LaTeX pipeline.
//!
//! * `tokenizer` — flat token stream (keywords, Greek, operators, …).
//! * `latex`     — walks the stream and emits a LaTeX expression.
//!
//! Input comes from `ir::body::EquationControl::script`. The MD
//! exporter wraps the output in `$$ … $$` math blocks.

pub mod latex;
pub mod tokenizer;

pub use latex::to_latex;
pub use tokenizer::{Greek, Keyword, Lexer, Op, Span, Token, TokenKind};
