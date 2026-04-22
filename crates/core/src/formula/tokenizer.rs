//! HWP equation script tokenizer.
//!
//! Grammar is whitespace-separated keyword soup with `{ }` groups, popularised
//! by the hml-equation-parser reference. Examples:
//!
//! ```text
//!   SUM FROM { i=1 } TO { n } { a _ { i } }
//!   LEFT ⌊ a + b RIGHT ⌋
//!   { 1 OVER 2 }
//!   MATRIX { 1 & 2 ## 3 & 4 }
//! ```
//!
//! One awkward case: `LEFT` / `RIGHT` are followed by a literal delimiter
//! character (which may be a unicode bracket). The lexer handles that by
//! flipping `pending_delim` so the next `advance()` emits `DelimChar(ch)`
//! regardless of what it looks like.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Amp,
    RowSep,
    Keyword(Keyword),
    Greek(Greek),
    Ident(String),
    Number(String),
    Operator(Op),
    DelimChar(char),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Over,
    Frac,
    Root,
    Sqrt,
    Sup,
    Sub,
    From,
    To,
    Sum,
    Prod,
    Int,
    Lim,
    Matrix,
    Pile,
    Left,
    Right,
    Mid,
    Bar,
    Hat,
    Tilde,
    Dot,
    DDot,
    Vector,
    Overline,
    Underline,
    Rm,
    Bf,
    It,
    Sin,
    Cos,
    Tan,
    Log,
    Ln,
    Exp,
    Cdot,
    Cdots,
    Ldots,
    Vdots,
    Approx,
    Equiv,
    Sim,
    Infty,
    Partial,
    Nabla,
    LArrow,
    LRArrow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Greek {
    Alpha,
    Beta,
    Gamma,
    Delta,
    Epsilon,
    Zeta,
    Eta,
    Theta,
    Iota,
    Kappa,
    Lambda,
    Mu,
    Nu,
    Xi,
    Omicron,
    Pi,
    Rho,
    Sigma,
    Tau,
    Upsilon,
    Phi,
    Chi,
    Psi,
    Omega,
    CapGamma,
    CapDelta,
    CapTheta,
    CapLambda,
    CapXi,
    CapPi,
    CapSigma,
    CapUpsilon,
    CapPhi,
    CapPsi,
    CapOmega,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    Lt,
    Gt,
    Bang,
    NotEq,
    LeEq,
    GeEq,
    PlusMinus,
}

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    pending_delim: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            pending_delim: false,
        }
    }

    pub fn tokenize(mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            let t = self.advance();
            let done = matches!(t.kind, TokenKind::Eof);
            out.push(t);
            if done {
                break;
            }
        }
        out
    }

    fn advance(&mut self) -> Token {
        self.skip_ws();
        let start = self.pos;

        let Some(&b) = self.bytes.get(self.pos) else {
            return Token { kind: TokenKind::Eof, span: Span { start, end: start } };
        };

        if self.pending_delim {
            self.pending_delim = false;
            let ch = self.src[self.pos..].chars().next().unwrap();
            self.pos += ch.len_utf8();
            return self.tok(TokenKind::DelimChar(ch), start);
        }

        match b {
            b'{' => self.single(TokenKind::LBrace, start),
            b'}' => self.single(TokenKind::RBrace, start),
            b'(' => self.single(TokenKind::LParen, start),
            b')' => self.single(TokenKind::RParen, start),
            b'[' => self.single(TokenKind::LBracket, start),
            b']' => self.single(TokenKind::RBracket, start),
            b'&' => self.single(TokenKind::Amp, start),
            b'#' => {
                if self.bytes.get(self.pos + 1) == Some(&b'#') {
                    self.pos += 2;
                    self.tok(TokenKind::RowSep, start)
                } else {
                    // Lone `#` is illegal in the HML grammar — pass through
                    // as a delimiter so the parser can surface a diagnostic.
                    self.pos += 1;
                    self.tok(TokenKind::DelimChar('#'), start)
                }
            }
            b'+' | b'-' | b'*' | b'/' | b'=' | b'<' | b'>' | b'!' => self.lex_op(start),
            b'0'..=b'9' | b'.' => self.lex_number(start),
            _ if b.is_ascii_alphabetic() => self.lex_word(start),
            _ => {
                let ch = self.src[self.pos..].chars().next().unwrap();
                self.pos += ch.len_utf8();
                self.tok(TokenKind::DelimChar(ch), start)
            }
        }
    }

    fn single(&mut self, kind: TokenKind, start: usize) -> Token {
        self.pos += 1;
        self.tok(kind, start)
    }

    fn tok(&self, kind: TokenKind, start: usize) -> Token {
        Token { kind, span: Span { start, end: self.pos } }
    }

    fn skip_ws(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn lex_op(&mut self, start: usize) -> Token {
        let a = self.bytes[self.pos];
        let nxt = self.bytes.get(self.pos + 1).copied();
        let (op, adv) = match (a, nxt) {
            (b'!', Some(b'=')) => (Op::NotEq, 2),
            (b'<', Some(b'=')) => (Op::LeEq, 2),
            (b'>', Some(b'=')) => (Op::GeEq, 2),
            (b'+', Some(b'-')) => (Op::PlusMinus, 2),
            (b'+', _) => (Op::Plus, 1),
            (b'-', _) => (Op::Minus, 1),
            (b'*', _) => (Op::Star, 1),
            (b'/', _) => (Op::Slash, 1),
            (b'=', _) => (Op::Eq, 1),
            (b'<', _) => (Op::Lt, 1),
            (b'>', _) => (Op::Gt, 1),
            (b'!', _) => (Op::Bang, 1),
            _ => unreachable!("lex_op dispatched on non-op byte"),
        };
        self.pos += adv;
        self.tok(TokenKind::Operator(op), start)
    }

    fn lex_number(&mut self, start: usize) -> Token {
        let mut end = self.pos;
        while let Some(&b) = self.bytes.get(end) {
            if b.is_ascii_digit() || b == b'.' {
                end += 1;
            } else {
                break;
            }
        }
        let s = self.src[self.pos..end].to_string();
        self.pos = end;
        self.tok(TokenKind::Number(s), start)
    }

    fn lex_word(&mut self, start: usize) -> Token {
        let mut end = self.pos;
        while let Some(&b) = self.bytes.get(end) {
            if b.is_ascii_alphabetic() {
                end += 1;
            } else {
                break;
            }
        }
        let word = &self.src[self.pos..end];
        self.pos = end;

        if let Some(g) = classify_greek(word) {
            return self.tok(TokenKind::Greek(g), start);
        }
        if let Some(kw) = classify_keyword(word) {
            if matches!(kw, Keyword::Left | Keyword::Right) {
                self.pending_delim = true;
            }
            return self.tok(TokenKind::Keyword(kw), start);
        }
        self.tok(TokenKind::Ident(word.to_string()), start)
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    fn next(&mut self) -> Option<Token> {
        let t = self.advance();
        if matches!(t.kind, TokenKind::Eof) {
            None
        } else {
            Some(t)
        }
    }
}

fn classify_keyword(w: &str) -> Option<Keyword> {
    use Keyword::*;
    Some(match w.to_ascii_lowercase().as_str() {
        "over" => Over,
        "frac" => Frac,
        "root" => Root,
        "sqrt" => Sqrt,
        "sup" => Sup,
        "sub" => Sub,
        "from" => From,
        "to" => To,
        "sum" => Sum,
        "prod" => Prod,
        "int" => Int,
        "lim" => Lim,
        "matrix" => Matrix,
        "pile" => Pile,
        "left" => Left,
        "right" => Right,
        "mid" => Mid,
        "bar" => Bar,
        "hat" => Hat,
        "tilde" => Tilde,
        "dot" => Dot,
        "ddot" => DDot,
        "vec" => Vector,
        "overline" => Overline,
        "underline" => Underline,
        "rm" => Rm,
        "bf" => Bf,
        "it" => It,
        "sin" => Sin,
        "cos" => Cos,
        "tan" => Tan,
        "log" => Log,
        "ln" => Ln,
        "exp" => Exp,
        "cdot" => Cdot,
        "cdots" => Cdots,
        "ldots" => Ldots,
        "vdots" => Vdots,
        "approx" => Approx,
        "equiv" => Equiv,
        "sim" => Sim,
        "infty" | "inf" => Infty,
        "partial" => Partial,
        "nabla" => Nabla,
        "leftarrow" => LArrow,
        "leftrightarrow" => LRArrow,
        _ => return None,
    })
}

fn classify_greek(w: &str) -> Option<Greek> {
    use Greek::*;
    Some(match w {
        "alpha" => Alpha,
        "beta" => Beta,
        "gamma" => Gamma,
        "delta" => Delta,
        "epsilon" => Epsilon,
        "zeta" => Zeta,
        "eta" => Eta,
        "theta" => Theta,
        "iota" => Iota,
        "kappa" => Kappa,
        "lambda" => Lambda,
        "mu" => Mu,
        "nu" => Nu,
        "xi" => Xi,
        "omicron" => Omicron,
        "pi" => Pi,
        "rho" => Rho,
        "sigma" => Sigma,
        "tau" => Tau,
        "upsilon" => Upsilon,
        "phi" => Phi,
        "chi" => Chi,
        "psi" => Psi,
        "omega" => Omega,
        "Gamma" => CapGamma,
        "Delta" => CapDelta,
        "Theta" => CapTheta,
        "Lambda" => CapLambda,
        "Xi" => CapXi,
        "Pi" => CapPi,
        "Sigma" => CapSigma,
        "Upsilon" => CapUpsilon,
        "Phi" => CapPhi,
        "Psi" => CapPsi,
        "Omega" => CapOmega,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        Lexer::new(src).tokenize().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn over_expression() {
        let k = kinds("{ 1 OVER 2 }");
        assert_eq!(k[0], TokenKind::LBrace);
        assert!(matches!(k[1], TokenKind::Number(ref n) if n == "1"));
        assert_eq!(k[2], TokenKind::Keyword(Keyword::Over));
        assert!(matches!(k[3], TokenKind::Number(ref n) if n == "2"));
        assert_eq!(k[4], TokenKind::RBrace);
        assert_eq!(k[5], TokenKind::Eof);
    }

    #[test]
    fn left_right_captures_delim() {
        let k = kinds("LEFT ⌊ a RIGHT ⌋");
        assert_eq!(k[0], TokenKind::Keyword(Keyword::Left));
        assert_eq!(k[1], TokenKind::DelimChar('⌊'));
        assert!(matches!(k[2], TokenKind::Ident(ref s) if s == "a"));
        assert_eq!(k[3], TokenKind::Keyword(Keyword::Right));
        assert_eq!(k[4], TokenKind::DelimChar('⌋'));
    }

    #[test]
    fn matrix_separators() {
        let k = kinds("MATRIX { 1 & 2 ## 3 & 4 }");
        assert!(k.contains(&TokenKind::Amp));
        assert!(k.contains(&TokenKind::RowSep));
    }

    #[test]
    fn greek_and_keyword() {
        let k = kinds("alpha + beta SUM");
        assert_eq!(k[0], TokenKind::Greek(Greek::Alpha));
        assert_eq!(k[1], TokenKind::Operator(Op::Plus));
        assert_eq!(k[2], TokenKind::Greek(Greek::Beta));
        assert_eq!(k[3], TokenKind::Keyword(Keyword::Sum));
    }
}
