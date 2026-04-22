//! HWP equation script → LaTeX pipeline.
//!
//! Stage 1 — tokenizer (this module's `tokenizer`).
//! Stage 2 — parser: flat token stream → `EqNode` tree (TODO).
//! Stage 3 — emitter: `EqNode` → LaTeX string wrapped in `$$ ... $$` (TODO).
//!
//! Input comes from `hwp::paragraph::control::equation::EquationRecord::script`.

pub mod tokenizer;

pub use tokenizer::{Greek, Keyword, Lexer, Op, Span, Token, TokenKind};
