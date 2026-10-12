//! The Best Practice Analyzer rule evaluator, as the earlier Python suite evaluated it:
//! the same text translation from Dynamic LINQ to Python, then the resulting Python
//! expression evaluated with Python's semantics.
//!
//! The supported Python subset covers what the translation can produce from the rule
//! language: `or`, `and`, `not`, comparison chains, unary `+` and `-`, parentheses,
//! string literals (adjacent ones concatenate), integer and float literals, `True`,
//! `False`, `None` and property names. Anything else is refused, as Python would raise
//! for most of it; `in`, `is`, arithmetic operators, attribute access and integer
//! literals beyond 64 bits are not modelled. The repository owner accepted that boundary
//! on 13 October 2026. Source layout follows `eval`: universal newlines, comments, line
//! joining and a line break ending the expression outside brackets.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

/// A value under Python's semantics.
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    None,
}

impl Datum {
    pub fn truthy(&self) -> bool {
        match self {
            Datum::Text(text) => !text.is_empty(),
            Datum::Int(number) => *number != 0,
            Datum::Float(number) => *number != 0.0,
            Datum::Bool(flag) => *flag,
            Datum::None => false,
        }
    }

    /// An int or bool (an int subclass) as an exact integer.
    fn integer(&self) -> Option<i64> {
        match self {
            Datum::Int(number) => Some(*number),
            Datum::Bool(flag) => Some(i64::from(*flag)),
            _ => None,
        }
    }
}

/// Python's exact comparison of an int with a finite or infinite float.
fn int_float_cmp(int: i64, float: f64) -> Ordering {
    const TWO_TO_63: f64 = 9_223_372_036_854_775_808.0;
    if float >= TWO_TO_63 {
        return Ordering::Less;
    }
    if float < -TWO_TO_63 {
        return Ordering::Greater;
    }
    let whole = float.floor();
    match int.cmp(&(whole as i64)) {
        Ordering::Equal if float > whole => Ordering::Less,
        other => other,
    }
}

/// The order of two numbers, compared exactly as Python compares ints and floats.
fn number_order(left: &Datum, right: &Datum) -> Option<Ordering> {
    match (left, right) {
        (Datum::Float(a), Datum::Float(b)) => a.partial_cmp(b),
        (Datum::Float(a), other) => other.integer().map(|b| int_float_cmp(b, *a).reverse()),
        (other, Datum::Float(b)) => other.integer().map(|a| int_float_cmp(a, *b)),
        _ => Some(left.integer()?.cmp(&right.integer()?)),
    }
}

pub type Object = HashMap<&'static str, Datum>;

pub fn text(value: &str) -> Datum {
    Datum::Text(value.to_owned())
}

/// The Python suite's translation, step for step: `string.IsNullOrEmpty(X)` becomes
/// `(not X)`, `&&`, `||` and `<>` become `and`, `or` and `!=`, and any `!` not before
/// `=` becomes `not`. Like the original it rewrites string literal contents too.
fn translate(expression: &str) -> String {
    static NULL_OR_EMPTY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)string\.IsNullOrEmpty\((\w+)\)").unwrap());
    let translated = NULL_OR_EMPTY.replace_all(expression, "(not $1)");
    let translated = translated
        .replace("&&", " and ")
        .replace("||", " or ")
        .replace("<>", "!=");
    let mut result = String::new();
    let mut chars = translated.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '!' && chars.peek() != Some(&'=') {
            result.push_str(" not ");
        } else {
            result.push(c);
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    Literal(Datum),
    Symbol(&'static str),
}

const SYMBOLS: [&str; 10] = ["==", "!=", "<=", ">=", "<", ">", "(", ")", "-", "+"];

/// The character an octal (`\ooo`, up to three digits) or hexadecimal (`\xhh`, `\uhhhh`,
/// `\Uhhhhhhhh`) escape names, reading the digits after `kind` from `chars`.
fn numeric_escape(kind: char, chars: &mut std::str::Chars<'_>) -> char {
    if let Some(first) = kind.to_digit(8) {
        let mut value = first;
        for _ in 0..2 {
            let Some(digit) = chars.clone().next().and_then(|c| c.to_digit(8)) else {
                break;
            };
            value = value * 8 + digit;
            chars.next();
        }
        return char::from_u32(value).expect("octal escape");
    }
    let width = match kind {
        'x' => 2,
        'u' => 4,
        _ => 8,
    };
    let hex: String = chars.by_ref().take(width).collect();
    let valid = hex.len() == width && hex.bytes().all(|b| b.is_ascii_hexdigit());
    let value = u32::from_str_radix(&hex, 16).ok().filter(|_| valid);
    value
        .and_then(char::from_u32)
        .expect("SyntaxError: truncated or invalid hexadecimal escape")
}

/// A Python string literal body with its escapes decoded; unknown escapes keep the backslash.
fn decode_string(body: &str) -> String {
    let mut result = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('a') => result.push('\x07'),
            Some('b') => result.push('\x08'),
            Some('f') => result.push('\x0c'),
            Some('v') => result.push('\x0b'),
            Some(quote @ ('\\' | '\'' | '"')) => result.push(quote),
            Some('\n') => {}
            Some(kind @ ('0'..='7' | 'x' | 'u' | 'U')) => result.push(numeric_escape(kind, &mut chars)),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => panic!("SyntaxError: unterminated string literal"),
        }
    }
    result
}

/// A Python numeric literal: an int (no leading zeros unless all zeros) or a float.
fn number_literal(raw: &str) -> Datum {
    let digits = raw.replace('_', "");
    if raw.contains(['.', 'e', 'E']) {
        return Datum::Float(digits.parse().expect("float literal"));
    }
    if digits.len() > 1 && digits.starts_with('0') && digits.bytes().any(|b| b != b'0') {
        panic!("SyntaxError: leading zeros in decimal integer literals are not permitted");
    }
    Datum::Int(
        digits
            .parse()
            .unwrap_or_else(|_| panic!("integer literal beyond 64 bits: {raw}")),
    )
}

/// Python's numeric literal grammar, ASCII digits with single underscores between them.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    let digits = r"[0-9](?:_?[0-9])*";
    let exponent = format!(r"(?:[eE][+-]?{digits})");
    let pattern = format!(
        r"^(?:(?:{digits})?\.{digits}{exponent}?|{digits}\.(?:{digits})?{exponent}?|{digits}{exponent}|{digits})"
    );
    Regex::new(&pattern).unwrap()
});
static STRING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(?:"(?:[^"\\\n]|\\(?s:.))*"|'(?:[^'\\\n]|\\(?s:.))*')"#).unwrap());
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\W\d]\w*").unwrap());

/// Tokens of Python source, read as `eval` reads it: universal newlines; spaces, tabs
/// and form feeds between tokens; `#` comments; a backslash before a line break joins
/// lines; and a line break outside brackets ends the expression, so only blank lines
/// and comments may follow it.
fn tokens(source: &str) -> Vec<Token> {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut found = Vec::new();
    let (mut depth, mut ended) = (0_usize, false);
    let mut rest = source.as_str();
    loop {
        rest = skip_layout(rest, depth > 0 || found.is_empty(), &mut ended);
        if rest.is_empty() {
            return found;
        }
        assert!(!ended, "SyntaxError: invalid syntax after a line break");
        let (token, length) = next_token(rest);
        match token {
            Token::Symbol("(") => depth += 1,
            Token::Symbol(")") => depth = depth.saturating_sub(1),
            _ => {}
        }
        found.push(token);
        rest = &rest[length..];
    }
}

/// Skips layout before the next token. A line break counts as an end of the
/// expression unless `open` (inside brackets, or before the first token).
fn skip_layout<'a>(mut rest: &'a str, open: bool, ended: &mut bool) -> &'a str {
    loop {
        if let Some(tail) = rest
            .strip_prefix([' ', '\t', '\x0c'])
            .or_else(|| rest.strip_prefix("\\\n"))
        {
            rest = tail;
        } else if rest.starts_with('#') {
            rest = rest.find('\n').map_or("", |at| &rest[at..]);
        } else if let Some(tail) = rest.strip_prefix('\n') {
            *ended |= !open;
            rest = tail;
        } else {
            return rest;
        }
    }
}

fn next_token(rest: &str) -> (Token, usize) {
    if let Some(m) = STRING.find(rest) {
        return (
            Token::Literal(Datum::Text(decode_string(&m.as_str()[1..m.len() - 1]))),
            m.len(),
        );
    }
    let number = NUMBER
        .find(rest)
        .filter(|m| !rest[m.end()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    if let Some(m) = number {
        return (Token::Literal(number_literal(m.as_str())), m.len());
    }
    if let Some(m) = NAME.find(rest) {
        let token = match m.as_str() {
            "True" => Token::Literal(Datum::Bool(true)),
            "False" => Token::Literal(Datum::Bool(false)),
            "None" => Token::Literal(Datum::None),
            word => Token::Name(word.to_owned()),
        };
        return (token, m.len());
    }
    let symbol = SYMBOLS.iter().find(|s| rest.starts_with(**s));
    symbol.map_or_else(
        || panic!("SyntaxError: unsupported text at {rest:?}"),
        |s| (Token::Symbol(s), s.len()),
    )
}

/// A parsed Python expression.
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// A comparison chain `a op b op c`, read as `a op b and b op c`.
    Compare(Box<Expr>, Vec<(&'static str, Expr)>),
    Negate(Box<Expr>),
    Plus(Box<Expr>),
    Name(String),
    Literal(Datum),
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn take(&mut self) -> Token {
        let token = self
            .tokens
            .get(self.at)
            .cloned()
            .expect("SyntaxError: unexpected end of expression");
        self.at += 1;
        token
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Name(found)) if found == word)
    }

    fn or(&mut self) -> Expr {
        let mut expr = self.and();
        while self.is_word("or") {
            self.take();
            expr = Expr::Or(Box::new(expr), Box::new(self.and()));
        }
        expr
    }

    fn and(&mut self) -> Expr {
        let mut expr = self.not();
        while self.is_word("and") {
            self.take();
            expr = Expr::And(Box::new(expr), Box::new(self.not()));
        }
        expr
    }

    fn not(&mut self) -> Expr {
        if self.is_word("not") {
            self.take();
            return Expr::Not(Box::new(self.not()));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Expr {
        let first = self.unary();
        let mut chain = Vec::new();
        while let Some(Token::Symbol(op @ ("==" | "!=" | "<" | "<=" | ">" | ">="))) = self.peek() {
            let op = *op;
            self.take();
            chain.push((op, self.unary()));
        }
        if chain.is_empty() {
            first
        } else {
            Expr::Compare(Box::new(first), chain)
        }
    }

    fn unary(&mut self) -> Expr {
        match self.peek() {
            Some(Token::Symbol("-")) => {
                self.take();
                Expr::Negate(Box::new(self.unary()))
            }
            Some(Token::Symbol("+")) => {
                self.take();
                Expr::Plus(Box::new(self.unary()))
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Expr {
        match self.take() {
            Token::Symbol("(") => {
                let expr = self.or();
                assert_eq!(self.take(), Token::Symbol(")"), "SyntaxError: '(' was never closed");
                expr
            }
            Token::Literal(Datum::Text(mut joined)) => {
                while let Some(Token::Literal(Datum::Text(next))) = self.peek() {
                    joined.push_str(next);
                    self.take();
                }
                Expr::Literal(Datum::Text(joined))
            }
            Token::Literal(value) => Expr::Literal(value),
            Token::Name(word) if !["and", "or", "not", "in", "is", "if", "else", "lambda"].contains(&word.as_str()) => {
                Expr::Name(word)
            }
            other => panic!("SyntaxError: unexpected {other:?}"),
        }
    }
}

/// A property; an undefined one fails like Python's NameError, only when reached.
fn property(object: &Object, name: &str) -> Datum {
    object
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("NameError: name '{name}' is not defined"))
}

fn compare(op: &str, left: &Datum, right: &Datum) -> bool {
    let equal = match (left, right) {
        (Datum::Text(a), Datum::Text(b)) => a == b,
        (Datum::None, Datum::None) => true,
        _ => number_order(left, right) == Some(Ordering::Equal),
    };
    let order = || match (left, right) {
        (Datum::Text(a), Datum::Text(b)) => a.cmp(b),
        _ => number_order(left, right)
            .unwrap_or_else(|| panic!("TypeError: '{op}' not supported between {left:?} and {right:?}")),
    };
    match op {
        "==" => equal,
        "!=" => !equal,
        "<" => order() == Ordering::Less,
        "<=" => order() != Ordering::Greater,
        ">" => order() == Ordering::Greater,
        _ => order() != Ordering::Less,
    }
}

fn negate(value: Datum) -> Datum {
    match value {
        Datum::Int(number) => Datum::Int(number.checked_neg().expect("integer beyond 64 bits")),
        Datum::Float(number) => Datum::Float(-number),
        Datum::Bool(flag) => Datum::Int(-i64::from(flag)),
        other => panic!("TypeError: bad operand type for unary -: {other:?}"),
    }
}

/// Python's evaluation: `and` and `or` return an operand and skip the right side once
/// the left side decides the result.
fn eval(expr: &Expr, object: &Object) -> Datum {
    match expr {
        Expr::Or(left, right) => {
            let value = eval(left, object);
            if value.truthy() { value } else { eval(right, object) }
        }
        Expr::And(left, right) => {
            let value = eval(left, object);
            if value.truthy() { eval(right, object) } else { value }
        }
        Expr::Not(inner) => Datum::Bool(!eval(inner, object).truthy()),
        Expr::Compare(first, chain) => {
            let mut left = eval(first, object);
            for (op, operand) in chain {
                let right = eval(operand, object);
                if !compare(op, &left, &right) {
                    return Datum::Bool(false);
                }
                left = right;
            }
            Datum::Bool(true)
        }
        Expr::Negate(inner) => negate(eval(inner, object)),
        Expr::Plus(inner) => match eval(inner, object) {
            Datum::Bool(flag) => Datum::Int(i64::from(flag)),
            number @ (Datum::Int(_) | Datum::Float(_)) => number,
            other => panic!("TypeError: bad operand type for unary +: {other:?}"),
        },
        Expr::Name(name) => property(object, name),
        Expr::Literal(value) => value.clone(),
    }
}

/// Evaluate one Dynamic LINQ rule expression against a single object's properties. The
/// whole translated expression is parsed first, so a syntax error fails even on a
/// branch the evaluation skips, as Python's compile step does.
pub fn evaluate(expression: &str, object: &Object) -> bool {
    let mut parser = Parser {
        tokens: tokens(&translate(expression)),
        at: 0,
    };
    let expr = parser.or();
    assert!(parser.peek().is_none(), "SyntaxError: invalid syntax in {expression}");
    eval(&expr, object).truthy()
}
