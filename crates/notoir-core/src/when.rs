//! Context ("when") expressions for command enablement and keybindings
//! (docs/specs/05-command-system-and-keybindings.md).
//!
//! Grammar: identifiers (`editorFocus`, `plugin.x.y`), `!`, `&&`, `||`, `==`,
//! `!=`, string literals (`'markdown'` / `"markdown"`), numbers, `true` /
//! `false`, parentheses. Parsed once at registration.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Str(String),
    Num(f64),
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Str(s) => !s.is_empty(),
            Value::Num(n) => *n != 0.0,
        }
    }
}

/// Context keys captured once per frame (or when the palette opens).
#[derive(Clone, Debug, Default)]
pub struct ContextSnapshot {
    values: HashMap<String, Value>,
}

impl ContextSnapshot {
    pub fn set(&mut self, key: impl Into<String>, value: Value) -> &mut Self {
        self.values.insert(key.into(), value);
        self
    }

    pub fn set_bool(&mut self, key: impl Into<String>, value: bool) -> &mut Self {
        self.set(key, Value::Bool(value))
    }

    pub fn set_str(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.set(key, Value::Str(value.into()))
    }

    pub fn set_num(&mut self, key: impl Into<String>, value: f64) -> &mut Self {
        self.set(key, Value::Num(value))
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Key(String),
    Lit(Value),
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Truthy(Operand),
    Eq(Operand, Operand),
    Ne(Operand, Operand),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operand::Key(k) => f.write_str(k),
            Operand::Lit(Value::Bool(b)) => write!(f, "{b}"),
            Operand::Lit(Value::Str(s)) => write!(f, "'{s}'"),
            Operand::Lit(Value::Num(n)) => write!(f, "{n}"),
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let wrap = |e: &Expr| match e {
            Expr::And(..) | Expr::Or(..) => format!("({e})"),
            _ => e.to_string(),
        };
        match self {
            Expr::Truthy(o) => write!(f, "{o}"),
            Expr::Eq(a, b) => write!(f, "{a} == {b}"),
            Expr::Ne(a, b) => write!(f, "{a} != {b}"),
            Expr::Not(e) => write!(f, "!{}", wrap(e)),
            Expr::And(a, b) => write!(f, "{} && {}", wrap(a), wrap(b)),
            Expr::Or(a, b) => write!(f, "{} || {}", wrap(a), wrap(b)),
        }
    }
}

impl std::fmt::Display for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(e) => write!(f, "{e}"),
            None => Ok(()),
        }
    }
}

/// A parsed context expression; the empty expression is always true.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct When(Option<Expr>);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid when-expression at {position}: {message}")]
pub struct WhenError {
    pub position: usize,
    pub message: &'static str,
}

impl When {
    pub const ALWAYS: When = When(None);

    pub fn parse(source: &str) -> Result<When, WhenError> {
        let tokens = tokenize(source)?;
        if tokens.is_empty() {
            return Ok(When(None));
        }
        let mut p = Parser { tokens, pos: 0 };
        let expr = p.or()?;
        if p.pos < p.tokens.len() {
            return Err(p.error("unexpected token"));
        }
        Ok(When(Some(expr)))
    }

    pub fn eval(&self, ctx: &ContextSnapshot) -> bool {
        self.0.as_ref().is_none_or(|e| eval(e, ctx))
    }
}

fn value_of(o: &Operand, ctx: &ContextSnapshot) -> Option<Value> {
    match o {
        Operand::Key(k) => ctx.get(k).cloned(),
        Operand::Lit(v) => Some(v.clone()),
    }
}

fn eval(e: &Expr, ctx: &ContextSnapshot) -> bool {
    match e {
        Expr::Truthy(o) => value_of(o, ctx).is_some_and(|v| v.truthy()),
        Expr::Eq(a, b) => value_of(a, ctx) == value_of(b, ctx),
        Expr::Ne(a, b) => value_of(a, ctx) != value_of(b, ctx),
        Expr::Not(inner) => !eval(inner, ctx),
        Expr::And(a, b) => eval(a, ctx) && eval(b, ctx),
        Expr::Or(a, b) => eval(a, ctx) || eval(b, ctx),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    Not,
    And,
    Or,
    Eq,
    Ne,
    LParen,
    RParen,
}

fn tokenize(src: &str) -> Result<Vec<(Tok, usize)>, WhenError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let err = |position, message| WhenError { position, message };
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        let two = |a: char, b: char| c == a && chars.get(i + 1) == Some(&b);
        let tok = if c.is_whitespace() {
            i += 1;
            continue;
        } else if two('&', '&') {
            i += 2;
            Tok::And
        } else if two('|', '|') {
            i += 2;
            Tok::Or
        } else if two('=', '=') {
            i += 2;
            Tok::Eq
        } else if two('!', '=') {
            i += 2;
            Tok::Ne
        } else if c == '!' {
            i += 1;
            Tok::Not
        } else if c == '(' {
            i += 1;
            Tok::LParen
        } else if c == ')' {
            i += 1;
            Tok::RParen
        } else if c == '\'' || c == '"' {
            let end = chars[i + 1..]
                .iter()
                .position(|x| *x == c)
                .ok_or(err(start, "unterminated string"))?;
            let s: String = chars[i + 1..i + 1 + end].iter().collect();
            i += end + 2;
            Tok::Str(s)
        } else if c.is_ascii_digit() {
            let len = chars[i..]
                .iter()
                .take_while(|x| x.is_ascii_digit() || **x == '.')
                .count();
            let s: String = chars[i..i + len].iter().collect();
            i += len;
            Tok::Num(s.parse().map_err(|_| err(start, "invalid number"))?)
        } else if c.is_alphabetic() || c == '_' {
            let len = chars[i..]
                .iter()
                .take_while(|x| x.is_alphanumeric() || matches!(x, '_' | '.' | '-' | ':'))
                .count();
            let s: String = chars[i..i + len].iter().collect();
            i += len;
            Tok::Ident(s)
        } else {
            return Err(err(start, "unexpected character"));
        };
        out.push((tok, start));
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<(Tok, usize)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    fn error(&self, message: &'static str) -> WhenError {
        let position = self.tokens.get(self.pos).map_or_else(
            || self.tokens.last().map_or(0, |(_, p)| *p + 1),
            |(_, p)| *p,
        );
        WhenError { position, message }
    }

    fn or(&mut self) -> Result<Expr, WhenError> {
        let mut left = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.pos += 1;
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, WhenError> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Tok::And) {
            self.pos += 1;
            left = Expr::And(Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, WhenError> {
        if self.peek() == Some(&Tok::Not) {
            self.pos += 1;
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.peek() == Some(&Tok::LParen) {
            self.pos += 1;
            let e = self.or()?;
            if self.peek() != Some(&Tok::RParen) {
                return Err(self.error("expected ')'"));
            }
            self.pos += 1;
            return Ok(e);
        }
        let left = self.operand()?;
        match self.peek() {
            Some(Tok::Eq) => {
                self.pos += 1;
                Ok(Expr::Eq(left, self.operand()?))
            }
            Some(Tok::Ne) => {
                self.pos += 1;
                Ok(Expr::Ne(left, self.operand()?))
            }
            _ => Ok(Expr::Truthy(left)),
        }
    }

    fn operand(&mut self) -> Result<Operand, WhenError> {
        let op = match self.peek() {
            Some(Tok::Ident(s)) if s == "true" => Operand::Lit(Value::Bool(true)),
            Some(Tok::Ident(s)) if s == "false" => Operand::Lit(Value::Bool(false)),
            Some(Tok::Ident(s)) => Operand::Key(s.clone()),
            Some(Tok::Str(s)) => Operand::Lit(Value::Str(s.clone())),
            Some(Tok::Num(n)) => Operand::Lit(Value::Num(*n)),
            _ => return Err(self.error("expected a key or value")),
        };
        self.pos += 1;
        Ok(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ContextSnapshot {
        let mut c = ContextSnapshot::default();
        c.set_bool("editorFocus", true)
            .set_bool("readOnly", false)
            .set_str("languageId", "markdown")
            .set_num("groupCount", 2.0);
        c
    }

    #[test]
    fn evaluates_expressions() {
        let c = ctx();
        let t = |s: &str| When::parse(s).unwrap().eval(&c);
        assert!(t(""));
        assert!(t("editorFocus && !readOnly"));
        assert!(t("languageId == 'markdown'"));
        assert!(!t("languageId != \"markdown\""));
        assert!(t("readOnly || (editorFocus && groupCount == 2)"));
        assert!(!t("missingKey"));
        assert!(t("!missingKey"));
        assert!(t("plugin.word-count.enabled || true"));
    }

    #[test]
    fn precedence_and_binds_tighter_than_or() {
        let c = ctx();
        assert!(
            When::parse("editorFocus || readOnly && readOnly")
                .unwrap()
                .eval(&c)
        );
    }

    #[test]
    fn invalid_expressions_report_position() {
        assert_eq!(
            When::parse("editorFocus &&"),
            Err(WhenError {
                position: 13,
                message: "expected a key or value"
            })
        );
        assert!(When::parse("(a").is_err());
        assert!(When::parse("a b").is_err());
        assert!(When::parse("'open").is_err());
        assert!(When::parse("a # b").is_err());
    }
}
