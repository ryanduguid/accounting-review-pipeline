//! The Best Practice Analyzer rule evaluator: Dynamic LINQ evaluated as the earlier
//! Python suite evaluated it, by translation to Python and `eval`.

use std::cmp::Ordering;
use std::collections::HashMap;

use regex::Regex;

/// A value under Python's semantics, which the earlier suite evaluated the rules with.
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    Text(String),
    Int(i64),
    Bool(bool),
}

impl Datum {
    pub fn truthy(&self) -> bool {
        match self {
            Datum::Text(text) => !text.is_empty(),
            Datum::Int(number) => *number != 0,
            Datum::Bool(flag) => *flag,
        }
    }

    fn number(&self) -> Option<i64> {
        match self {
            Datum::Int(number) => Some(*number),
            Datum::Bool(flag) => Some(i64::from(*flag)),
            Datum::Text(_) => None,
        }
    }
}

pub type Object = HashMap<&'static str, Datum>;

pub fn text(value: &str) -> Datum {
    Datum::Text(value.to_owned())
}

/// A parsed rule expression in the Dynamic LINQ subset the rules use: `||`, `&&`, `!`,
/// comparisons (`<>` is `!=`), string and integer literals, property names and
/// `string.IsNullOrEmpty(Property)`.
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// A comparison chain `a op b op c`, which Python reads as `a op b and b op c`.
    Compare(Box<Expr>, Vec<(String, Expr)>),
    IsNullOrEmpty(String),
    Property(String),
    Literal(Datum),
}

struct Parser {
    tokens: Vec<String>,
    at: usize,
}

fn tokens(expression: &str) -> Vec<String> {
    let token =
        Regex::new(r#"(?i:string\.IsNullOrEmpty)|"[^"]*"|\d+|[A-Za-z_]\w*|&&|\|\||<>|!=|==|<=|>=|[!<>()]|\S"#).unwrap();
    token.find_iter(expression).map(|m| m.as_str().to_owned()).collect()
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.at).map(String::as_str)
    }

    fn take(&mut self) -> String {
        let token = self.tokens.get(self.at).cloned().expect("unexpected end of expression");
        self.at += 1;
        token
    }

    fn expect(&mut self, wanted: &str) {
        let found = self.take();
        assert_eq!(found, wanted, "malformed rule expression");
    }

    fn or(&mut self) -> Expr {
        let mut expr = self.and();
        while self.peek() == Some("||") {
            self.take();
            expr = Expr::Or(Box::new(expr), Box::new(self.and()));
        }
        expr
    }

    fn and(&mut self) -> Expr {
        let mut expr = self.not();
        while self.peek() == Some("&&") {
            self.take();
            expr = Expr::And(Box::new(expr), Box::new(self.not()));
        }
        expr
    }

    fn not(&mut self) -> Expr {
        if self.peek() == Some("!") {
            self.take();
            return Expr::Not(Box::new(self.not()));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Expr {
        let left = self.primary();
        let operators = ["==", "!=", "<>", "<", "<=", ">", ">="];
        let mut chain = Vec::new();
        while let Some(op) = self.peek().filter(|t| operators.contains(t)).map(str::to_owned) {
            self.take();
            chain.push((op, self.primary()));
        }
        if chain.is_empty() {
            left
        } else {
            Expr::Compare(Box::new(left), chain)
        }
    }

    fn primary(&mut self) -> Expr {
        let token = self.take();
        if token.eq_ignore_ascii_case("string.IsNullOrEmpty") {
            self.expect("(");
            let name = self.take();
            self.expect(")");
            return Expr::IsNullOrEmpty(name);
        }
        if token == "(" {
            let expr = self.or();
            self.expect(")");
            return expr;
        }
        if let Some(literal) = token.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
            return Expr::Literal(text(literal));
        }
        if let Ok(number) = token.parse() {
            return Expr::Literal(Datum::Int(number));
        }
        Expr::Property(token)
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
    let order = || match (left, right) {
        (Datum::Text(a), Datum::Text(b)) => a.cmp(b),
        _ => match (left.number(), right.number()) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => panic!("TypeError: '{op}' not supported between {left:?} and {right:?}"),
        },
    };
    let equal = left == right || matches!((left.number(), right.number()), (Some(a), Some(b)) if a == b);
    match op {
        "==" => equal,
        "!=" | "<>" => !equal,
        "<" => order() == Ordering::Less,
        "<=" => order() != Ordering::Greater,
        ">" => order() == Ordering::Greater,
        _ => order() != Ordering::Less,
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
        Expr::IsNullOrEmpty(name) => Datum::Bool(!property(object, name).truthy()),
        Expr::Property(name) => property(object, name),
        Expr::Literal(value) => value.clone(),
    }
}

/// Evaluate one Dynamic LINQ rule expression against a single object's properties. The
/// whole expression is parsed first, so a syntax error fails even on a branch the
/// evaluation skips, as Python's compile step does.
pub fn evaluate(expression: &str, object: &Object) -> bool {
    let mut parser = Parser {
        tokens: tokens(expression),
        at: 0,
    };
    let expr = parser.or();
    assert!(parser.peek().is_none(), "trailing tokens in {expression}");
    eval(&expr, object).truthy()
}
