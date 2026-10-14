//! Small expression language for user functions (computed columns).
//!
//! ```text
//! [Freq] / 1e9                      column names in brackets
//! S21db - [Thru Loss]               simple names can be written directly
//! db20(sqrt(a^2 + b^2))             functions, ^ is the power operator
//! ```

use super::dataset::Dataset;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    /// Column reference (or constant name, resolved at evaluation).
    Ident(String),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// Functions available in expressions: (name, arity, description).
pub const FUNCTIONS: &[(&str, usize, &str)] = &[
    ("abs", 1, "absolute value"),
    ("sqrt", 1, "square root"),
    ("exp", 1, "e^x"),
    ("ln", 1, "natural logarithm"),
    ("log10", 1, "base-10 logarithm"),
    ("log", 1, "base-10 logarithm"),
    ("log2", 1, "base-2 logarithm"),
    ("sin", 1, "sine (radians)"),
    ("cos", 1, "cosine (radians)"),
    ("tan", 1, "tangent (radians)"),
    ("asin", 1, "arc sine"),
    ("acos", 1, "arc cosine"),
    ("atan", 1, "arc tangent"),
    ("atan2", 2, "atan2(y, x)"),
    ("deg", 1, "radians → degrees"),
    ("rad", 1, "degrees → radians"),
    ("floor", 1, "round down"),
    ("ceil", 1, "round up"),
    ("round", 1, "round to nearest"),
    ("sign", 1, "sign (-1, 0, 1)"),
    ("min", 2, "minimum of two values"),
    ("max", 2, "maximum of two values"),
    ("pow", 2, "pow(x, y) = x^y"),
    ("hypot", 2, "sqrt(x² + y²)"),
    ("db10", 1, "10·log10(x)  (power ratio → dB)"),
    ("db20", 1, "20·log10(|x|) (voltage ratio → dB)"),
    ("lin10", 1, "10^(x/10)    (dB → power ratio)"),
    ("lin20", 1, "10^(x/20)    (dB → voltage ratio)"),
    ("dbm2w", 1, "dBm → W"),
    ("w2dbm", 1, "W → dBm"),
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            let v = text.parse::<f64>().map_err(|_| format!("Invalid number '{text}'"))?;
            out.push(Tok::Num(v));
        } else if c == '[' {
            // column name, may itself contain brackets: [VDD Current [A]]
            let start = i + 1;
            let mut depth = 1;
            let mut j = start;
            while j < chars.len() {
                match chars[j] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j >= chars.len() {
                return Err("Missing ']' after column name".to_owned());
            }
            out.push(Tok::Ident(chars[start..j].iter().collect::<String>().trim().to_owned()));
            i = j + 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            match c {
                '+' | '-' | '*' | '/' | '^' => out.push(Tok::Op(c)),
                '(' => out.push(Tok::LParen),
                ')' => out.push(Tok::RParen),
                ',' | ';' => out.push(Tok::Comma),
                _ => return Err(format!("Unexpected character '{c}'")),
            }
            i += 1;
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.term()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let rhs = self.term()?;
            let op = if c == '+' { BinOp::Add } else { BinOp::Sub };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut lhs = self.unary()?;
        while let Some(Tok::Op(c @ ('*' | '/'))) = self.peek().cloned() {
            self.pos += 1;
            let rhs = self.unary()?;
            let op = if c == '*' { BinOp::Mul } else { BinOp::Div };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Tok::Op('+')) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.pos += 1;
            let exp = self.unary()?;
            return Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(v)) => Ok(Expr::Num(v)),
            Some(Tok::LParen) => {
                let e = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("Missing ')'".to_owned()),
                }
            }
            Some(Tok::Ident(name)) => {
                if let Some(Tok::LParen) = self.peek() {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if let Some(Tok::RParen) = self.peek() {
                        self.pos += 1;
                    } else {
                        loop {
                            args.push(self.expr()?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => return Err(format!("Missing ')' after arguments of {name}()")),
                            }
                        }
                    }
                    let lname = name.to_ascii_lowercase();
                    let Some((_, arity, _)) = FUNCTIONS.iter().find(|(n, _, _)| *n == lname) else {
                        return Err(format!("Unknown function '{name}'"));
                    };
                    if *arity != args.len() {
                        return Err(format!("{name}() expects {arity} argument(s), got {}", args.len()));
                    }
                    Ok(Expr::Call(lname, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Some(t) => Err(format!("Unexpected {}", describe(&t))),
            None => Err("Unexpected end of expression".to_owned()),
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(v) => format!("number {v}"),
        Tok::Ident(s) => format!("name '{s}'"),
        Tok::Op(c) => format!("operator '{c}'"),
        Tok::LParen => "'('".to_owned(),
        Tok::RParen => "')'".to_owned(),
        Tok::Comma => "','".to_owned(),
    }
}

pub fn parse(src: &str) -> Result<Expr, String> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err("Empty expression".to_owned());
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.expr()?;
    if p.pos < p.toks.len() {
        return Err(format!("Unexpected {}", describe(&p.toks[p.pos])));
    }
    Ok(e)
}

enum Val {
    S(f64),
    V(Vec<f64>),
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" | "PI" | "Pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        _ => None,
    }
}

fn map1(v: Val, f: impl Fn(f64) -> f64) -> Val {
    match v {
        Val::S(a) => Val::S(f(a)),
        Val::V(mut a) => {
            a.iter_mut().for_each(|x| *x = f(*x));
            Val::V(a)
        }
    }
}

fn map2(a: Val, b: Val, f: impl Fn(f64, f64) -> f64) -> Val {
    match (a, b) {
        (Val::S(x), Val::S(y)) => Val::S(f(x, y)),
        (Val::V(mut x), Val::S(y)) => {
            x.iter_mut().for_each(|v| *v = f(*v, y));
            Val::V(x)
        }
        (Val::S(x), Val::V(mut y)) => {
            y.iter_mut().for_each(|v| *v = f(x, *v));
            Val::V(y)
        }
        (Val::V(mut x), Val::V(y)) => {
            x.iter_mut().zip(y).for_each(|(a, b)| *a = f(*a, b));
            Val::V(x)
        }
    }
}

fn eval_val(e: &Expr, ds: &Dataset) -> Result<Val, String> {
    Ok(match e {
        Expr::Num(v) => Val::S(*v),
        Expr::Ident(name) => match ds.column(name) {
            Some(col) => Val::V((0..ds.n_rows).map(|r| col.value(r)).collect()),
            None => match constant(name) {
                Some(c) => Val::S(c),
                None => return Err(format!("Unknown column '{name}'")),
            },
        },
        Expr::Neg(a) => map1(eval_val(a, ds)?, |x| -x),
        Expr::Bin(op, a, b) => {
            let a = eval_val(a, ds)?;
            let b = eval_val(b, ds)?;
            match op {
                BinOp::Add => map2(a, b, |x, y| x + y),
                BinOp::Sub => map2(a, b, |x, y| x - y),
                BinOp::Mul => map2(a, b, |x, y| x * y),
                BinOp::Div => map2(a, b, |x, y| x / y),
                BinOp::Pow => map2(a, b, f64::powf),
            }
        }
        Expr::Call(name, args) => {
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                vals.push(eval_val(a, ds)?);
            }
            if vals.len() == 2 {
                let b = vals.pop().unwrap_or(Val::S(f64::NAN));
                let a = vals.pop().unwrap_or(Val::S(f64::NAN));
                return Ok(match name.as_str() {
                    "atan2" => map2(a, b, f64::atan2),
                    "min" => map2(a, b, f64::min),
                    "max" => map2(a, b, f64::max),
                    "pow" => map2(a, b, f64::powf),
                    "hypot" => map2(a, b, f64::hypot),
                    _ => return Err(format!("Unknown function '{name}'")),
                });
            }
            let a = vals.pop().unwrap_or(Val::S(f64::NAN));
            let f: fn(f64) -> f64 = match name.as_str() {
                "abs" => f64::abs,
                "sqrt" => f64::sqrt,
                "exp" => f64::exp,
                "ln" => f64::ln,
                "log10" | "log" => f64::log10,
                "log2" => f64::log2,
                "sin" => f64::sin,
                "cos" => f64::cos,
                "tan" => f64::tan,
                "asin" => f64::asin,
                "acos" => f64::acos,
                "atan" => f64::atan,
                "deg" => f64::to_degrees,
                "rad" => f64::to_radians,
                "floor" => f64::floor,
                "ceil" => f64::ceil,
                "round" => f64::round,
                "sign" => |x: f64| {
                    if x > 0.0 {
                        1.0
                    } else if x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                },
                "db10" => |x: f64| 10.0 * x.log10(),
                "db20" => |x: f64| 20.0 * x.abs().log10(),
                "lin10" => |x: f64| 10f64.powf(x / 10.0),
                "lin20" => |x: f64| 10f64.powf(x / 20.0),
                "dbm2w" => |x: f64| 10f64.powf((x - 30.0) / 10.0),
                "w2dbm" => |x: f64| 10.0 * x.log10() + 30.0,
                _ => return Err(format!("Unknown function '{name}'")),
            };
            map1(a, f)
        }
    })
}

/// Evaluates an expression on every row of a dataset.
pub fn evaluate(e: &Expr, ds: &Dataset) -> Result<Vec<f64>, String> {
    Ok(match eval_val(e, ds)? {
        Val::S(v) => vec![v; ds.n_rows],
        Val::V(v) => v,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::loader::{ImportOptions, parse_csv};

    #[test]
    fn parse_and_eval() {
        let ds = parse_csv(
            b"Freq,S21db,Thru Loss,I [A]\n1e9,10,1,0.5\n2e9,20,2,0.25\n",
            &ImportOptions::default(),
        )
        .unwrap();
        let e = parse("[I [A]] * 1000").unwrap();
        assert_eq!(evaluate(&e, &ds).unwrap(), vec![500.0, 250.0]);
        let e = parse("[Freq] / 1e9").unwrap();
        assert_eq!(evaluate(&e, &ds).unwrap(), vec![1.0, 2.0]);
        let e = parse("S21db - [Thru Loss]*2").unwrap();
        assert_eq!(evaluate(&e, &ds).unwrap(), vec![8.0, 16.0]);
        let e = parse("-2^2 + max(S21db, 15)").unwrap();
        assert_eq!(evaluate(&e, &ds).unwrap(), vec![11.0, 16.0]);
        let e = parse("db10(100)").unwrap();
        assert_eq!(evaluate(&e, &ds).unwrap(), vec![20.0, 20.0]);
        assert!(parse("Freq +").is_err());
        assert!(parse("foo(1)").is_err());
        assert!(evaluate(&parse("Nope * 2").unwrap(), &ds).is_err());
        let e = parse("[Thru Loss] + S21db * pi").unwrap();
        assert_eq!(evaluate(&e, &ds).unwrap()[0], 1.0 + 10.0 * std::f64::consts::PI);
    }
}
