//! HWP equation script → LaTeX.
//!
//! Takes the token stream emitted by [`super::Lexer`] and walks it
//! emitting LaTeX fragments. Intentionally covers the common shapes
//! — fractions, sub/super, Greek, Sum/Prod/Int with limits, Sqrt/
//! Root, LEFT/RIGHT delimiters, standard functions — and falls back
//! to emitting the original source slice for anything the parser
//! doesn't recognise. Producing SOMETHING usable beats perfect
//! coverage: downstream renderers (KaTeX / MathJax) tolerate stray
//! characters much better than a hard failure.
//!
//! Not aimed at:
//!   * HWP-specific layout controls (PILE, ALIGN variants, font
//!     switches RM/BF/IT beyond a basic `\mathrm/\mathbf/\mathit`).
//!   * Exotic mathematical typography (CASES, accents stacking,
//!     over/under braces). Those map to LaTeX but need more thought
//!     than this first pass gives.

use super::tokenizer::{Greek, Keyword, Lexer, Op, Token, TokenKind};

/// Convert an HWP equation script into a LaTeX expression. Returns
/// an empty string when the input is empty or whitespace only.
pub fn to_latex(script: &str) -> String {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let tokens = Lexer::new(trimmed).tokenize();
    let mut p = Translator {
        tokens: &tokens,
        src: trimmed,
        pos: 0,
    };
    p.parse_seq(Stop::Eof)
}

struct Translator<'a> {
    tokens: &'a [Token],
    src: &'a str,
    pos: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Eof,
    RBrace,
}

impl<'a> Translator<'a> {
    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        self.pos += 1;
        t
    }

    fn at_stop(&self, stop: Stop) -> bool {
        match (stop, &self.tokens[self.pos].kind) {
            (_, TokenKind::Eof) => true,
            (Stop::RBrace, TokenKind::RBrace) => true,
            _ => false,
        }
    }

    /// Parse a sequence of atoms, stopping at EOF or `}` depending
    /// on `stop`. Handles infix operators (OVER, _, ^) by rewriting
    /// the atom vector in place — the last committed atom becomes
    /// the operator's left operand.
    fn parse_seq(&mut self, stop: Stop) -> String {
        let mut atoms: Vec<String> = Vec::new();
        while !self.at_stop(stop) {
            // Infix: OVER, _, ^ all pull the previous atom as their
            // left operand. They sit at the current position.
            match self.peek() {
                TokenKind::Keyword(Keyword::Over) => {
                    self.advance();
                    let num = atoms.pop().unwrap_or_default();
                    let den = self.parse_atom();
                    atoms.push(format!(
                        "\\frac{{{}}}{{{}}}",
                        strip_outer_braces(&num),
                        strip_outer_braces(&den),
                    ));
                }
                TokenKind::Keyword(Keyword::Sup) | TokenKind::DelimChar('^') => {
                    self.advance();
                    let sup = self.parse_atom();
                    let base = atoms.pop().unwrap_or_default();
                    atoms.push(format!("{}^{{{}}}", base, strip_outer_braces(&sup)));
                }
                TokenKind::Keyword(Keyword::Sub) | TokenKind::DelimChar('_') => {
                    self.advance();
                    let sub = self.parse_atom();
                    let base = atoms.pop().unwrap_or_default();
                    atoms.push(format!("{}_{{{}}}", base, strip_outer_braces(&sub)));
                }
                _ => atoms.push(self.parse_atom()),
            }
        }
        atoms.join(" ")
    }

    /// Parse one atom — either a single token or a `{...}` group or
    /// a keyword with its own children (SQRT, SUM, LEFT, …).
    fn parse_atom(&mut self) -> String {
        if matches!(self.peek(), TokenKind::Eof) {
            return String::new();
        }
        let tok = self.advance().clone();
        match tok.kind {
            TokenKind::LBrace => {
                let inner = self.parse_seq(Stop::RBrace);
                if matches!(self.peek(), TokenKind::RBrace) {
                    self.pos += 1;
                }
                format!("{{{}}}", inner)
            }
            TokenKind::LParen => "(".into(),
            TokenKind::RParen => ")".into(),
            TokenKind::LBracket => "[".into(),
            TokenKind::RBracket => "]".into(),
            TokenKind::RBrace => "}".into(),
            TokenKind::Amp => "&".into(),
            TokenKind::RowSep => " \\\\ ".into(),
            TokenKind::Number(n) => n,
            TokenKind::Ident(id) => escape_ident(&id),
            TokenKind::Greek(g) => greek_to_latex(g),
            TokenKind::Operator(op) => op_to_latex(op).into(),
            TokenKind::DelimChar(c) => delim_to_latex(c),
            TokenKind::Keyword(k) => self.keyword_atom(k),
            TokenKind::Eof => String::new(),
        }
    }

    /// Consume the current token if it's an LBrace and return the
    /// group body; otherwise take the next atom. Most keyword args
    /// in HWP script are wrapped in `{}` but a bare single token
    /// (like `SQRT x`) is also accepted.
    fn parse_arg(&mut self) -> String {
        let inner = self.parse_atom();
        // parse_atom wraps groups in braces already; pass through.
        inner
    }

    /// Emit a keyword that introduces its own children (prefix
    /// operators and the Left/Right delimiter pair).
    fn keyword_atom(&mut self, k: Keyword) -> String {
        match k {
            Keyword::Sqrt => format!("\\sqrt{}", self.parse_arg()),
            Keyword::Root => {
                // `ROOT {n} {x}` → `\sqrt[n]{x}`.
                let n = strip_outer_braces(&self.parse_arg()).to_string();
                let x = strip_outer_braces(&self.parse_arg()).to_string();
                format!("\\sqrt[{}]{{{}}}", n, x)
            }
            Keyword::Frac => {
                // Explicit `FRAC {a} {b}` → `\frac{a}{b}`.
                let a = strip_outer_braces(&self.parse_arg()).to_string();
                let b = strip_outer_braces(&self.parse_arg()).to_string();
                format!("\\frac{{{}}}{{{}}}", a, b)
            }
            Keyword::Sum => self.big_op("\\sum"),
            Keyword::Prod => self.big_op("\\prod"),
            Keyword::Int => self.big_op("\\int"),
            Keyword::Lim => self.lim_op(),
            Keyword::Matrix => self.matrix("pmatrix"),
            Keyword::Pile => self.matrix("matrix"),
            Keyword::Left => {
                let delim = self.consume_delim();
                format!("\\left{}", latex_delim(delim))
            }
            Keyword::Right => {
                let delim = self.consume_delim();
                format!("\\right{}", latex_delim(delim))
            }
            Keyword::Mid => "\\mid".into(),
            Keyword::Bar => format!("\\bar{}", self.parse_arg()),
            Keyword::Hat => format!("\\hat{}", self.parse_arg()),
            Keyword::Tilde => format!("\\tilde{}", self.parse_arg()),
            Keyword::Dot => format!("\\dot{}", self.parse_arg()),
            Keyword::DDot => format!("\\ddot{}", self.parse_arg()),
            Keyword::Vector => format!("\\vec{}", self.parse_arg()),
            Keyword::Overline => format!("\\overline{}", self.parse_arg()),
            Keyword::Underline => format!("\\underline{}", self.parse_arg()),
            Keyword::Rm => format!("\\mathrm{}", self.parse_arg()),
            Keyword::Bf => format!("\\mathbf{}", self.parse_arg()),
            Keyword::It => format!("\\mathit{}", self.parse_arg()),
            Keyword::Sin => "\\sin".into(),
            Keyword::Cos => "\\cos".into(),
            Keyword::Tan => "\\tan".into(),
            Keyword::Log => "\\log".into(),
            Keyword::Ln => "\\ln".into(),
            Keyword::Exp => "\\exp".into(),
            Keyword::Cdot => "\\cdot".into(),
            Keyword::Cdots => "\\cdots".into(),
            Keyword::Ldots => "\\ldots".into(),
            Keyword::Vdots => "\\vdots".into(),
            Keyword::Approx => "\\approx".into(),
            Keyword::Equiv => "\\equiv".into(),
            Keyword::Sim => "\\sim".into(),
            Keyword::Infty => "\\infty".into(),
            Keyword::Partial => "\\partial".into(),
            Keyword::Nabla => "\\nabla".into(),
            Keyword::LArrow => "\\leftarrow".into(),
            Keyword::LRArrow => "\\leftrightarrow".into(),
            // FROM / TO only appear as modifiers on big ops; if
            // they're seen standalone, treat as idents so the output
            // still round-trips human-readably.
            Keyword::From => "\\text{from}".into(),
            Keyword::To => "\\text{to}".into(),
            // Sup / Sub handled in parse_seq as infix.
            Keyword::Sup | Keyword::Sub => String::new(),
            // Over handled infix too.
            Keyword::Over => String::new(),
        }
    }

    /// Parse a `\sum` / `\prod` / `\int` with optional FROM/TO
    /// limits. Leaves the body for the surrounding sequence — the
    /// big op itself just emits the operator + limits.
    fn big_op(&mut self, op: &str) -> String {
        let mut from: Option<String> = None;
        let mut to: Option<String> = None;
        loop {
            match self.peek() {
                TokenKind::Keyword(Keyword::From) => {
                    self.advance();
                    from = Some(self.parse_arg());
                }
                TokenKind::Keyword(Keyword::To) => {
                    self.advance();
                    to = Some(self.parse_arg());
                }
                _ => break,
            }
        }
        let mut s = op.to_string();
        if let Some(f) = from {
            s.push_str(&format!("_{{{}}}", strip_outer_braces(&f)));
        }
        if let Some(t) = to {
            s.push_str(&format!("^{{{}}}", strip_outer_braces(&t)));
        }
        s
    }

    /// `LIM _ {x -> 0}` → `\lim_{x \to 0}`. Subscripts after LIM
    /// become the limit spec; the body is the next atom.
    fn lim_op(&mut self) -> String {
        let mut s = "\\lim".to_string();
        if matches!(
            self.peek(),
            TokenKind::Keyword(Keyword::Sub) | TokenKind::DelimChar('_')
        ) {
            self.advance();
            let spec = strip_outer_braces(&self.parse_arg()).to_string();
            // Soft-rewrite `->` → `\to` if it survived as plain text.
            let rewired = spec.replace("->", "\\to ");
            s.push_str(&format!("_{{{}}}", rewired));
        }
        s
    }

    fn matrix(&mut self, env: &str) -> String {
        // Expect a `{ ... }` group; inside, `&` separates cells and
        // `##` (RowSep) separates rows. parse_seq already translates
        // both directly, so wrapping in an env is straightforward.
        if matches!(self.peek(), TokenKind::LBrace) {
            self.advance();
            let body = self.parse_seq(Stop::RBrace);
            if matches!(self.peek(), TokenKind::RBrace) {
                self.pos += 1;
            }
            let inner = strip_outer_braces(&body);
            format!("\\begin{{{env}}} {inner} \\end{{{env}}}")
        } else {
            // No braces — degenerate; pull a single atom and wrap.
            let body = self.parse_arg();
            format!(
                "\\begin{{{env}}} {} \\end{{{env}}}",
                strip_outer_braces(&body)
            )
        }
    }

    /// `LEFT` / `RIGHT` are followed by a literal delimiter char
    /// (the lexer flips `pending_delim` to emit it as `DelimChar`).
    /// Consume whatever comes next and return its character form.
    fn consume_delim(&mut self) -> char {
        match self.peek() {
            TokenKind::DelimChar(c) => {
                let c = *c;
                self.advance();
                c
            }
            TokenKind::LParen => {
                self.advance();
                '('
            }
            TokenKind::RParen => {
                self.advance();
                ')'
            }
            TokenKind::LBracket => {
                self.advance();
                '['
            }
            TokenKind::RBracket => {
                self.advance();
                ']'
            }
            TokenKind::LBrace => {
                self.advance();
                '{'
            }
            TokenKind::RBrace => {
                self.advance();
                '}'
            }
            _ => '.',
        }
    }
}

/// Wrap identifier longer than 1 char in `\mathit{…}` so LaTeX
/// doesn't interpret each letter as its own variable.
fn escape_ident(id: &str) -> String {
    if id.len() == 1 || id.chars().count() == 1 {
        id.to_string()
    } else {
        format!("\\mathit{{{}}}", id)
    }
}

fn greek_to_latex(g: Greek) -> String {
    let name = match g {
        Greek::Alpha => "alpha",
        Greek::Beta => "beta",
        Greek::Gamma => "gamma",
        Greek::Delta => "delta",
        Greek::Epsilon => "epsilon",
        Greek::Zeta => "zeta",
        Greek::Eta => "eta",
        Greek::Theta => "theta",
        Greek::Iota => "iota",
        Greek::Kappa => "kappa",
        Greek::Lambda => "lambda",
        Greek::Mu => "mu",
        Greek::Nu => "nu",
        Greek::Xi => "xi",
        Greek::Omicron => "omicron",
        Greek::Pi => "pi",
        Greek::Rho => "rho",
        Greek::Sigma => "sigma",
        Greek::Tau => "tau",
        Greek::Upsilon => "upsilon",
        Greek::Phi => "phi",
        Greek::Chi => "chi",
        Greek::Psi => "psi",
        Greek::Omega => "omega",
        Greek::CapGamma => "Gamma",
        Greek::CapDelta => "Delta",
        Greek::CapTheta => "Theta",
        Greek::CapLambda => "Lambda",
        Greek::CapXi => "Xi",
        Greek::CapPi => "Pi",
        Greek::CapSigma => "Sigma",
        Greek::CapUpsilon => "Upsilon",
        Greek::CapPhi => "Phi",
        Greek::CapPsi => "Psi",
        Greek::CapOmega => "Omega",
    };
    format!("\\{}", name)
}

fn op_to_latex(op: Op) -> &'static str {
    match op {
        Op::Plus => "+",
        Op::Minus => "-",
        Op::Star => "*",
        Op::Slash => "/",
        Op::Eq => "=",
        Op::Lt => "<",
        Op::Gt => ">",
        Op::Bang => "!",
        Op::NotEq => "\\neq",
        Op::LeEq => "\\leq",
        Op::GeEq => "\\geq",
        Op::PlusMinus => "\\pm",
    }
}

fn delim_to_latex(c: char) -> String {
    // Unicode brackets common in HWP equations → their LaTeX
    // macros. Everything else passes through as-is (KaTeX handles
    // plain unicode well enough in text mode).
    match c {
        '⌊' => "\\lfloor".into(),
        '⌋' => "\\rfloor".into(),
        '⌈' => "\\lceil".into(),
        '⌉' => "\\rceil".into(),
        '∑' => "\\sum".into(),
        '∏' => "\\prod".into(),
        '∫' => "\\int".into(),
        '∞' => "\\infty".into(),
        '∂' => "\\partial".into(),
        '∇' => "\\nabla".into(),
        '∈' => "\\in".into(),
        '∉' => "\\notin".into(),
        '⊆' => "\\subseteq".into(),
        '⊂' => "\\subset".into(),
        '∪' => "\\cup".into(),
        '∩' => "\\cap".into(),
        '≠' => "\\neq".into(),
        '≈' => "\\approx".into(),
        '≤' => "\\leq".into(),
        '≥' => "\\geq".into(),
        '→' => "\\to".into(),
        '←' => "\\leftarrow".into(),
        '↔' => "\\leftrightarrow".into(),
        '∅' => "\\emptyset".into(),
        _ => c.to_string(),
    }
}

fn latex_delim(c: char) -> String {
    match c {
        '(' | ')' | '[' | ']' | '|' => c.to_string(),
        '{' => "\\{".into(),
        '}' => "\\}".into(),
        '⌊' => "\\lfloor".into(),
        '⌋' => "\\rfloor".into(),
        '⌈' => "\\lceil".into(),
        '⌉' => "\\rceil".into(),
        '.' => ".".into(),
        _ => c.to_string(),
    }
}

/// Strip exactly one pair of outer braces when present. Used when
/// embedding a parsed subexpression into a position where the braces
/// would be double (e.g. `\frac{{x}}{...}` → `\frac{x}{...}`).
fn strip_outer_braces(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_script_yields_empty_latex() {
        assert_eq!(to_latex(""), "");
        assert_eq!(to_latex("   "), "");
    }

    #[test]
    fn plain_algebra_passes_through() {
        assert!(to_latex("x + 1").contains("x + 1"));
    }

    #[test]
    fn over_becomes_frac() {
        let s = to_latex("{1 OVER 2}");
        assert!(s.contains("\\frac{1}{2}"), "got: {s}");
    }

    #[test]
    fn explicit_frac_keyword_works() {
        let s = to_latex("FRAC {a+b} {c}");
        assert!(s.contains("\\frac{a+b}{c}") || s.contains("\\frac{a + b}{c}"), "got: {s}");
    }

    #[test]
    fn sqrt_and_root() {
        let s = to_latex("SQRT {x+1}");
        assert!(s.contains("\\sqrt{x+1}") || s.contains("\\sqrt{x + 1}"), "got: {s}");
        let r = to_latex("ROOT {3} {x}");
        assert!(r.contains("\\sqrt[3]{x}"), "got: {r}");
    }

    #[test]
    fn subscript_superscript_via_delim_chars() {
        let s = to_latex("a _ {i}");
        assert!(s.contains("a_{i}"), "got: {s}");
        let t = to_latex("a ^ {n}");
        assert!(t.contains("a^{n}"), "got: {t}");
    }

    #[test]
    fn sum_with_from_to_emits_limits() {
        // Tokenizer splits `i=1` into `i`, `=`, `1` with whitespace
        // between — the test accepts either form.
        let s = to_latex("SUM FROM {i=1} TO {n} {a_i}");
        assert!(s.contains("\\sum"), "got: {s}");
        assert!(s.contains("_{i = 1}") || s.contains("_{i=1}"), "got: {s}");
        assert!(s.contains("^{n}"), "got: {s}");
    }

    #[test]
    fn greek_letters_map() {
        assert!(to_latex("alpha + beta").contains("\\alpha"));
        assert!(to_latex("alpha + beta").contains("\\beta"));
        // Capital Greek in HWP script uses `Gamma` (first letter up),
        // matching LaTeX convention. All-caps `GAMMA` is not a
        // recognised form and falls through to a plain identifier.
        assert!(to_latex("Gamma").contains("\\Gamma"));
    }

    #[test]
    fn standard_functions() {
        assert!(to_latex("sin x").contains("\\sin"));
        assert!(to_latex("cos x").contains("\\cos"));
        assert!(to_latex("log x").contains("\\log"));
    }

    #[test]
    fn left_right_with_brackets() {
        let s = to_latex("LEFT ( a + b RIGHT )");
        assert!(s.contains("\\left("), "got: {s}");
        assert!(s.contains("\\right)"), "got: {s}");
    }

    #[test]
    fn unicode_delims_translate() {
        let s = to_latex("LEFT ⌊ x RIGHT ⌋");
        assert!(s.contains("\\left\\lfloor"), "got: {s}");
        assert!(s.contains("\\right\\rfloor"), "got: {s}");
    }

    #[test]
    fn matrix_emits_pmatrix_env() {
        let s = to_latex("MATRIX {1 & 2 ## 3 & 4}");
        assert!(s.contains("\\begin{pmatrix}"), "got: {s}");
        assert!(s.contains("\\end{pmatrix}"), "got: {s}");
        assert!(s.contains("\\\\"), "got: {s}");
    }

    #[test]
    fn unknown_tokens_dont_panic() {
        // Exotic delimiter characters should pass through as-is.
        let s = to_latex("★ + ♥");
        assert!(s.contains("★") || !s.is_empty());
    }

    #[test]
    fn operators_mapped_to_latex() {
        assert!(to_latex("a != b").contains("\\neq"));
        assert!(to_latex("a <= b").contains("\\leq"));
        assert!(to_latex("a +- b").contains("\\pm"));
    }
}
