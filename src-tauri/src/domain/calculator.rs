use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, Weekday};

pub const MAX_INPUT: usize = 2_000;
pub const MAX_LINES: usize = 40;
pub const MAX_DEPTH: usize = 32;
pub const MAX_ARGUMENTS: usize = 20;
pub const LARGEST: f64 = 1e15;
pub const MOST_DECIMALS: i64 = 10;
pub const MOST_DAYS: i64 = 1_000_000;
const SIGNIFICANT: usize = 12;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Fault {
    #[error("nothing to work out")]
    Empty,
    #[error("the input is {0} characters long, and the most is {MAX_INPUT}")]
    TooLong(usize),
    #[error("the input has {0} lines, and the most is {MAX_LINES}")]
    TooManyLines(usize),
    #[error("it nests deeper than {MAX_DEPTH} levels")]
    TooDeep,
    #[error("division by zero")]
    DivisionByZero,
    #[error("the result is too large: past {LARGEST:e}, or not a number at all")]
    Overflow,
    #[error("{0}")]
    Domain(String),
    #[error("{0}")]
    Malformed(String),
    #[error("{0} is not a date")]
    BadDate(String),
    #[error("no number or function is called `{0}`")]
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalcError {
    pub line: usize,
    pub said: String,
    pub fault: Fault,
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.fault)
        } else {
            write!(f, "line {}, `{}`: {}", self.line, self.said, self.fault)
        }
    }
}

impl std::error::Error for CalcError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Number(f64),
    Date(NaiveDate),
    Day(Weekday),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => f.write_str(&number_words(*n)),
            Value::Date(date) => write!(f, "{}, a {}", date.format("%Y-%m-%d"), day_name(date.weekday())),
            Value::Day(day) => f.write_str(day_name(*day)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Worked {
    pub said: String,
    pub name: Option<String>,
    pub value: Value,
}

impl std::fmt::Display for Worked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name} = {}", self.value),
            None => write!(f, "{} = {}", self.said, self.value),
        }
    }
}

pub fn day_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

pub fn clean(n: f64) -> f64 {
    if n == 0.0 || !n.is_finite() {
        return if n == 0.0 { 0.0 } else { n };
    }
    format!("{:.*e}", SIGNIFICANT - 1, n).parse().unwrap_or(n)
}

pub fn number_words(n: f64) -> String {
    let n = clean(n);
    if n == 0.0 { "0".to_string() } else { format!("{n}") }
}

pub fn evaluate(input: &str) -> Result<Vec<Worked>, CalcError> {
    let whole = |fault| CalcError { line: 0, said: String::new(), fault };
    let length = input.chars().count();
    if length > MAX_INPUT {
        return Err(whole(Fault::TooLong(length)));
    }
    let statements: Vec<&str> =
        input.split(['\n', ';']).map(str::trim).filter(|line| !line.is_empty()).collect();
    if statements.is_empty() {
        return Err(whole(Fault::Empty));
    }
    if statements.len() > MAX_LINES {
        return Err(whole(Fault::TooManyLines(statements.len())));
    }
    let mut names: HashMap<String, Value> = HashMap::new();
    let mut out = Vec::with_capacity(statements.len());
    for (at, said) in statements.into_iter().enumerate() {
        let failed = |fault| CalcError { line: at + 1, said: said.to_string(), fault };
        let (name, value) = statement(said, &names).map_err(failed)?;
        if let Some(name) = &name {
            names.insert(name.clone(), value);
        }
        out.push(Worked { said: said.to_string(), name, value });
    }
    Ok(out)
}

fn statement(said: &str, names: &HashMap<String, Value>) -> Result<(Option<String>, Value), Fault> {
    let tokens = lex(said)?;
    let (name, body) = match tokens.as_slice() {
        [Token::Name(name), Token::Assign, rest @ ..] => {
            if FUNCTIONS.contains(&name.as_str()) || CONSTANTS.contains(&name.as_str()) {
                return Err(Fault::Malformed(format!(
                    "`{name}` is a built-in name; give the result another name"
                )));
            }
            (Some(name.clone()), rest)
        }
        _ => (None, tokens.as_slice()),
    };
    let mut parser = Parser { tokens: body, at: 0, depth: 0, names };
    let value = parser.expression()?;
    if let Some(left) = parser.tokens.get(parser.at) {
        return Err(Fault::Malformed(match left {
            Token::Comma => "a comma only separates a function's arguments: write 35000, not 35,000"
                .to_string(),
            Token::Assign => "only a name goes before `=`, as in `payment = 35000 / 60`".to_string(),
            Token::Close => "a `)` with no `(` before it".to_string(),
            other => format!("{} where an operator was expected", other.words()),
        }));
    }
    Ok((name, value))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Date(NaiveDate),
    Name(String),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Percent,
    Open,
    Close,
    Comma,
    Assign,
}

impl Token {
    fn words(&self) -> String {
        match self {
            Token::Number(n) => format!("the number {}", number_words(*n)),
            Token::Date(date) => format!("the date {}", date.format("%Y-%m-%d")),
            Token::Name(name) => format!("`{name}`"),
            Token::Plus => "`+`".into(),
            Token::Minus => "`-`".into(),
            Token::Times => "`*`".into(),
            Token::Divide => "`/`".into(),
            Token::Power => "`^`".into(),
            Token::Percent => "`%`".into(),
            Token::Open => "`(`".into(),
            Token::Close => "`)`".into(),
            Token::Comma => "`,`".into(),
            Token::Assign => "`=`".into(),
        }
    }
}

fn lex(said: &str) -> Result<Vec<Token>, Fault> {
    let chars: Vec<char> = said.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(at + 1).is_some_and(char::is_ascii_digit)) {
            let (token, next) = number_or_date(&chars, at)?;
            tokens.push(token);
            at = next;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = at;
            while at < chars.len() && (chars[at].is_ascii_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            tokens.push(Token::Name(chars[start..at].iter().collect::<String>().to_lowercase()));
            continue;
        }
        let token = match c {
            '+' => Token::Plus,
            '-' | '−' | '–' => Token::Minus,
            '*' if chars.get(at + 1) == Some(&'*') => {
                at += 1;
                Token::Power
            }
            '*' | '×' | '·' => Token::Times,
            '/' | '÷' => Token::Divide,
            '^' => Token::Power,
            '%' => Token::Percent,
            '(' => Token::Open,
            ')' => Token::Close,
            ',' => Token::Comma,
            '=' => Token::Assign,
            '$' | '€' | '£' | '¥' => {
                return Err(Fault::Malformed(format!(
                    "`{c}` is not part of a sum: leave out currency signs and units"
                )));
            }
            other => return Err(Fault::Malformed(format!("`{other}` is not something it can read"))),
        };
        tokens.push(token);
        at += 1;
    }
    Ok(tokens)
}

fn number_or_date(chars: &[char], start: usize) -> Result<(Token, usize), Fault> {
    let digits = |from: usize, count: usize| {
        (from..from + count).all(|i| chars.get(i).is_some_and(char::is_ascii_digit))
    };
    if digits(start, 4)
        && chars.get(start + 4) == Some(&'-')
        && digits(start + 5, 2)
        && chars.get(start + 7) == Some(&'-')
        && digits(start + 8, 2)
        && !chars.get(start + 10).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '.')
    {
        let words: String = chars[start..start + 10].iter().collect();
        let date = NaiveDate::parse_from_str(&words, "%Y-%m-%d")
            .map_err(|_| Fault::BadDate(words.clone()))?;
        return Ok((Token::Date(date), start + 10));
    }
    let mut at = start;
    while at < chars.len() && chars[at].is_ascii_digit() {
        at += 1;
    }
    if chars.get(at) == Some(&'.') {
        at += 1;
        while at < chars.len() && chars[at].is_ascii_digit() {
            at += 1;
        }
    }
    if matches!(chars.get(at), Some('e' | 'E')) {
        let sign = usize::from(matches!(chars.get(at + 1), Some('+' | '-')));
        if chars.get(at + 1 + sign).is_some_and(char::is_ascii_digit) {
            at += 1 + sign;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
        }
    }
    let words: String = chars[start..at].iter().collect();
    if chars.get(at).is_some_and(|c| c.is_ascii_alphabetic() || *c == '.') {
        return Err(Fault::Malformed(format!(
            "`{words}{}` is not a number",
            chars[at]
        )));
    }
    let n: f64 = words.parse().map_err(|_| Fault::Malformed(format!("`{words}` is not a number")))?;
    Ok((Token::Number(checked(n)?), at))
}

const FUNCTIONS: &[&str] = &[
    "pow", "sqrt", "ln", "log", "exp", "abs", "round", "floor", "ceil", "min", "max", "weekday",
    "days",
];
const CONSTANTS: &[&str] = &["pi", "e"];

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    depth: usize,
    names: &'a HashMap<String, Value>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn deeper(&mut self) -> Result<(), Fault> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(Fault::TooDeep) } else { Ok(()) }
    }

    fn expression(&mut self) -> Result<Value, Fault> {
        self.deeper()?;
        let mut left = self.term()?;
        loop {
            if self.eat(&Token::Plus) {
                left = add(left, self.term()?)?;
            } else if self.eat(&Token::Minus) {
                left = subtract(left, self.term()?)?;
            } else {
                break;
            }
        }
        self.depth -= 1;
        Ok(left)
    }

    fn term(&mut self) -> Result<Value, Fault> {
        let mut left = self.unary()?;
        loop {
            if self.eat(&Token::Times) {
                left = Value::Number(checked(number(left)? * number(self.unary()?)?)?);
            } else if self.eat(&Token::Divide) {
                let right = number(self.unary()?)?;
                if right == 0.0 {
                    return Err(Fault::DivisionByZero);
                }
                left = Value::Number(checked(number(left)? / right)?);
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Value, Fault> {
        if self.eat(&Token::Minus) {
            self.deeper()?;
            let value = Value::Number(-number(self.unary()?)?);
            self.depth -= 1;
            return Ok(value);
        }
        if self.eat(&Token::Plus) {
            self.deeper()?;
            let value = Value::Number(number(self.unary()?)?);
            self.depth -= 1;
            return Ok(value);
        }
        self.power()
    }

    fn power(&mut self) -> Result<Value, Fault> {
        let base = self.postfix()?;
        if self.eat(&Token::Power) {
            self.deeper()?;
            let exponent = number(self.unary()?)?;
            self.depth -= 1;
            return raise(number(base)?, exponent).map(Value::Number);
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<Value, Fault> {
        let mut value = self.primary()?;
        while self.eat(&Token::Percent) {
            value = Value::Number(number(value)? / 100.0);
        }
        Ok(value)
    }

    fn primary(&mut self) -> Result<Value, Fault> {
        let Some(token) = self.peek().cloned() else {
            return Err(Fault::Malformed("it ends where a number was expected".to_string()));
        };
        self.at += 1;
        match token {
            Token::Number(n) => Ok(Value::Number(n)),
            Token::Date(date) => Ok(Value::Date(date)),
            Token::Open => {
                let inner = self.expression()?;
                if !self.eat(&Token::Close) {
                    return Err(Fault::Malformed("a `(` is never closed".to_string()));
                }
                Ok(inner)
            }
            Token::Name(name) => {
                if self.eat(&Token::Open) {
                    let arguments = self.arguments()?;
                    return call(&name, &arguments);
                }
                match name.as_str() {
                    "pi" => Ok(Value::Number(std::f64::consts::PI)),
                    "e" => Ok(Value::Number(std::f64::consts::E)),
                    _ if FUNCTIONS.contains(&name.as_str()) => Err(Fault::Malformed(format!(
                        "`{name}` is a function: write `{name}(...)`"
                    ))),
                    _ => self.names.get(&name).copied().ok_or(Fault::Unknown(name)),
                }
            }
            other => Err(Fault::Malformed(format!(
                "{} where a number was expected",
                other.words()
            ))),
        }
    }

    fn arguments(&mut self) -> Result<Vec<Value>, Fault> {
        let mut out = Vec::new();
        if self.eat(&Token::Close) {
            return Ok(out);
        }
        loop {
            out.push(self.expression()?);
            if out.len() > MAX_ARGUMENTS {
                return Err(Fault::Malformed(format!(
                    "a function takes at most {MAX_ARGUMENTS} arguments"
                )));
            }
            if self.eat(&Token::Close) {
                return Ok(out);
            }
            if !self.eat(&Token::Comma) {
                return Err(Fault::Malformed("a `(` is never closed".to_string()));
            }
        }
    }
}

fn checked(n: f64) -> Result<f64, Fault> {
    if n.is_finite() && n.abs() <= LARGEST { Ok(n) } else { Err(Fault::Overflow) }
}

fn number(value: Value) -> Result<f64, Fault> {
    match value {
        Value::Number(n) => Ok(n),
        Value::Date(date) => Err(Fault::Malformed(format!(
            "the date {} cannot go in that sum: a date takes only + or - a number of days, - another \
             date, weekday(...) or days(...)",
            date.format("%Y-%m-%d")
        ))),
        Value::Day(day) => Err(Fault::Malformed(format!(
            "{} is a day of the week, not a number",
            day_name(day)
        ))),
    }
}

fn whole_days(n: f64) -> Result<i64, Fault> {
    let n = clean(n);
    if n.fract() != 0.0 || n.abs() > MOST_DAYS as f64 {
        return Err(Fault::Domain(format!(
            "a date moves by a whole number of days, at most {MOST_DAYS}, not {}",
            number_words(n)
        )));
    }
    Ok(n as i64)
}

fn shift(date: NaiveDate, days: i64) -> Result<Value, Fault> {
    date.checked_add_signed(Duration::days(days))
        .map(Value::Date)
        .ok_or_else(|| Fault::Domain("that date is out of range".to_string()))
}

fn add(left: Value, right: Value) -> Result<Value, Fault> {
    match (left, right) {
        (Value::Date(date), Value::Number(n)) | (Value::Number(n), Value::Date(date)) => {
            shift(date, whole_days(n)?)
        }
        (left, right) => Ok(Value::Number(checked(number(left)? + number(right)?)?)),
    }
}

fn subtract(left: Value, right: Value) -> Result<Value, Fault> {
    match (left, right) {
        (Value::Date(date), Value::Number(n)) => shift(date, -whole_days(n)?),
        (Value::Date(later), Value::Date(earlier)) => {
            Ok(Value::Number((later - earlier).num_days() as f64))
        }
        (left, right) => Ok(Value::Number(checked(number(left)? - number(right)?)?)),
    }
}

fn raise(base: f64, exponent: f64) -> Result<f64, Fault> {
    if base == 0.0 && exponent < 0.0 {
        return Err(Fault::DivisionByZero);
    }
    if base < 0.0 && exponent.fract() != 0.0 {
        return Err(Fault::Domain(
            "a negative number has no real power with a fractional exponent".to_string(),
        ));
    }
    checked(base.powf(exponent))
}

fn call(name: &str, arguments: &[Value]) -> Result<Value, Fault> {
    let wants = |count: &str| {
        Fault::Malformed(format!("`{name}` takes {count}, and was given {}", arguments.len()))
    };
    let one = || match arguments {
        [only] => number(*only),
        _ => Err(wants("one number")),
    };
    let numbers = || arguments.iter().map(|value| number(*value)).collect::<Result<Vec<_>, _>>();
    let n = match name {
        "pow" => match arguments {
            [base, exponent] => raise(number(*base)?, number(*exponent)?)?,
            _ => return Err(wants("two numbers, pow(base, exponent)")),
        },
        "sqrt" => {
            let x = one()?;
            if x < 0.0 {
                return Err(Fault::Domain("a negative number has no square root".to_string()));
            }
            x.sqrt()
        }
        "ln" | "log" => {
            let x = one()?;
            if x <= 0.0 {
                return Err(Fault::Domain(format!(
                    "`{name}` needs a number above zero, not {}",
                    number_words(x)
                )));
            }
            if name == "ln" { x.ln() } else { x.log10() }
        }
        "exp" => checked(one()?.exp())?,
        "abs" => one()?.abs(),
        "floor" => clean(one()?).floor(),
        "ceil" => clean(one()?).ceil(),
        "round" => {
            let (x, places) = match numbers()?.as_slice() {
                [x] => (*x, 0.0),
                [x, places] => (*x, *places),
                _ => return Err(wants("a number and, if you like, how many decimals")),
            };
            if places.fract() != 0.0 || !(0.0..=MOST_DECIMALS as f64).contains(&places) {
                return Err(Fault::Domain(format!(
                    "`round` keeps from 0 to {MOST_DECIMALS} decimals, not {}",
                    number_words(places)
                )));
            }
            let scale = 10f64.powi(places as i32);
            clean(x * scale).round() / scale
        }
        "min" | "max" => {
            let all = numbers()?;
            if all.is_empty() {
                return Err(wants("at least one number"));
            }
            let pick: fn(f64, f64) -> f64 = if name == "min" { f64::min } else { f64::max };
            all.into_iter().reduce(pick).unwrap_or_default()
        }
        "weekday" => {
            return match arguments {
                [Value::Date(date)] => Ok(Value::Day(date.weekday())),
                _ => Err(wants("one date written as YYYY-MM-DD, as in weekday(2026-12-20)")),
            };
        }
        "days" => {
            return match arguments {
                [Value::Date(from), Value::Date(to)] => {
                    Ok(Value::Number((*to - *from).num_days() as f64))
                }
                _ => Err(wants("two dates written as YYYY-MM-DD, as in days(2026-11-23, 2026-12-20)")),
            };
        }
        "pi" | "e" => return Err(wants("nothing, and is written without brackets")),
        _ => return Err(Fault::Unknown(name.to_string())),
    };
    Ok(Value::Number(checked(n)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(input: &str) -> String {
        let worked = evaluate(input).unwrap_or_else(|err| panic!("{input}: {err}"));
        worked.last().unwrap().value.to_string()
    }

    fn fault(input: &str) -> Fault {
        evaluate(input).expect_err(input).fault
    }

    #[test]
    fn a_division_by_zero_is_refused_wherever_it_happens() {
        for input in ["1 / 0", "5 / (2 - 2)", "pow(0, -1)", "0 ^ -2", "x = 0; 3 / x"] {
            assert_eq!(fault(input), Fault::DivisionByZero, "{input}");
        }
    }

    #[test]
    fn a_result_past_the_largest_is_an_overflow_and_not_a_number() {
        for input in ["10 ^ 16", "pow(2, 60)", "exp(1000)", "1e300 * 1e300", "999999999999999 * 10", "1e16"] {
            assert_eq!(fault(input), Fault::Overflow, "{input}");
        }
        assert_eq!(one("10 ^ 15"), "1000000000000000");
    }

    #[test]
    fn a_malformed_expression_says_what_is_wrong() {
        let malformed = |input: &str| match fault(input) {
            Fault::Malformed(words) => words,
            other => panic!("{input}: {other:?}"),
        };
        assert!(malformed("35,000 * 2").contains("not 35,000"));
        assert!(malformed("$500 * 36").contains("currency"));
        assert!(malformed("(1 + 2").contains("never closed"));
        assert!(malformed("1 + 2)").contains("no `(`"));
        assert!(malformed("1 +").contains("ends where a number"));
        assert!(malformed("2 3").contains("where an operator"));
        assert!(malformed("pow(2)").contains("two numbers"));
        assert!(malformed("sqrt").contains("is a function"));
        assert!(malformed("round = 2").contains("built-in"));
        assert!(malformed("1 = 2").contains("only a name"));
        assert!(malformed("3 # 4").contains("`#`"));
        assert!(malformed("12abc").contains("not a number"));
        assert!(malformed("2026-12-20 * 2").contains("cannot go in that sum"));
        assert!(malformed("weekday(2026-12-20) + 1").contains("day of the week"));
        assert!(malformed("weekday(20261220)").contains("YYYY-MM-DD"));
        assert_eq!(fault("loan * 2"), Fault::Unknown("loan".into()));
        assert_eq!(fault("cbrt(8)"), Fault::Unknown("cbrt".into()));
        assert_eq!(fault("weekday(2026-02-30)"), Fault::BadDate("2026-02-30".into()));
        assert_eq!(fault("   ;\n "), Fault::Empty);
    }

    #[test]
    fn a_number_outside_a_function_domain_is_refused() {
        for input in ["sqrt(-4)", "ln(0)", "log(-1)", "(-8) ^ 0.5", "round(2.5, 11)", "round(2.5, 1.5)", "2026-01-01 + 0.5"] {
            assert!(matches!(fault(input), Fault::Domain(_)), "{input}: {:?}", fault(input));
        }
    }

    #[test]
    fn the_input_and_its_nesting_are_bounded() {
        let long = "1+".repeat(MAX_INPUT / 2) + "1";
        assert_eq!(fault(&long), Fault::TooLong(long.chars().count()));
        let fits = "1+".repeat(MAX_INPUT / 2 - 1) + "1";
        assert_eq!(one(&fits), (MAX_INPUT / 2).to_string());
        let lines = vec!["1"; MAX_LINES + 1].join("\n");
        assert_eq!(fault(&lines), Fault::TooManyLines(MAX_LINES + 1));
        let deep = "(".repeat(MAX_DEPTH + 1) + "1" + &")".repeat(MAX_DEPTH + 1);
        assert_eq!(fault(&deep), Fault::TooDeep);
        assert_eq!(fault(&"-".repeat(MAX_DEPTH + 1).replace("", " ")), Fault::TooDeep);
        let shallow = "(".repeat(MAX_DEPTH - 1) + "1" + &")".repeat(MAX_DEPTH - 1);
        assert_eq!(one(&shallow), "1");
    }

    #[test]
    fn an_error_names_the_line_it_happened_on() {
        let err = evaluate("a = 2\nb = a / 0").unwrap_err();
        assert_eq!(err.line, 2);
        assert_eq!(err.to_string(), "line 2, `b = a / 0`: division by zero");
    }

    #[test]
    fn precedence_is_the_one_taught_in_school() {
        assert_eq!(one("2 + 3 * 4"), "14");
        assert_eq!(one("(2 + 3) * 4"), "20");
        assert_eq!(one("10 - 4 - 3"), "3");
        assert_eq!(one("100 / 10 / 5"), "2");
        assert_eq!(one("2 ^ 3 ^ 2"), "512");
        assert_eq!(one("2 ** 10"), "1024");
        assert_eq!(one("-2 ^ 2"), "-4");
        assert_eq!(one("(-2) ^ 2"), "4");
        assert_eq!(one("2 ^ -1"), "0.5");
        assert_eq!(one("6 × 7 − 2 ÷ 4"), "41.5");
        assert_eq!(one("--3"), "3");
    }

    #[test]
    fn results_carry_no_binary_noise() {
        assert_eq!(one("0.1 + 0.2"), "0.3");
        assert_eq!(one("1.1 * 1.1"), "1.21");
        assert_eq!(one("0.3 - 0.1"), "0.2");
        assert_eq!(one("1 / 3"), "0.333333333333");
        assert_eq!(one("2 / 3 * 3"), "2");
        assert_eq!(one("-0.0 * 5"), "0");
        assert_eq!(one("1e-20"), "0.00000000000000000001");
    }

    #[test]
    fn rounding_goes_half_away_from_zero_on_the_number_as_written() {
        assert_eq!(one("round(1.005, 2)"), "1.01");
        assert_eq!(one("round(2.5)"), "3");
        assert_eq!(one("round(-2.5)"), "-3");
        assert_eq!(one("round(676.649276, 2)"), "676.65");
        assert_eq!(one("round(19625.5)"), "19626");
        assert_eq!(one("floor(2.9) + ceil(2.1)"), "5");
        assert_eq!(one("floor(0.1 * 3 * 10)"), "3");
    }

    #[test]
    fn a_percent_is_a_hundredth() {
        assert_eq!(one("6%"), "0.06");
        assert_eq!(one("35000 * 6%"), "2100");
        assert_eq!(one("200 * (1 + 10%)"), "220");
        assert_eq!(one("6% / 12"), "0.005");
    }

    #[test]
    fn functions_cover_what_a_loan_or_compound_interest_needs() {
        assert_eq!(one("pow(1.05, 2)"), "1.1025");
        assert_eq!(one("sqrt(144)"), "12");
        assert_eq!(one("ln(e)"), "1");
        assert_eq!(one("exp(0)"), "1");
        assert_eq!(one("log(1000)"), "3");
        assert_eq!(one("min(3, 1, 2) + max(3, 1, 2)"), "4");
        assert_eq!(one("abs(-4.5)"), "4.5");
        assert_eq!(one("round(pi, 4)"), "3.1416");
        assert_eq!(one("round(10000 * exp(5% * 10), 2)"), "16487.21");
    }

    #[test]
    fn a_loan_balance_worked_out_in_named_steps_matches_the_known_answer() {
        let worked = evaluate(
            "rate = 6% / 12\n\
             payment = 35000 * rate / (1 - (1 + rate) ^ -60)\n\
             paid = payment * 36\n\
             owed = payment * (1 - (1 + rate) ^ -24) / rate\n\
             buy = paid + owed - 20000\n\
             lease = 3000 + 500 * 36\n\
             round(buy); round(owed); round(payment, 2)",
        )
        .unwrap();
        let shown: Vec<String> = worked.iter().map(ToString::to_string).collect();
        assert_eq!(shown[0], "rate = 0.005");
        assert_eq!(shown[1], "payment = 676.64805353");
        assert_eq!(shown[5], "lease = 21000");
        assert_eq!(shown[6], "round(buy) = 19626");
        assert_eq!(shown[7], "round(owed) = 15267");
        assert_eq!(shown[8], "round(payment, 2) = 676.65");
    }

    #[test]
    fn the_weekday_of_a_date_is_the_calendar_one() {
        assert_eq!(one("weekday(2026-12-20)"), "Sunday");
        assert_eq!(one("weekday(2026-11-23)"), "Monday");
        assert_eq!(one("weekday(2024-02-29)"), "Thursday");
        assert_eq!(one("weekday(2000-02-29)"), "Tuesday");
        assert_eq!(fault("weekday(2026-02-29)"), Fault::BadDate("2026-02-29".into()));
        assert_eq!(fault("weekday(1900-02-29)"), Fault::BadDate("1900-02-29".into()));
    }

    #[test]
    fn days_between_dates_count_forward_and_a_date_moves_by_whole_days() {
        assert_eq!(one("days(2026-11-23, 2026-12-20)"), "27");
        assert_eq!(one("2026-12-20 - 2026-11-23"), "27");
        assert_eq!(one("days(2026-12-20, 2026-11-23)"), "-27");
        assert_eq!(one("days(2024-02-28, 2024-03-01)"), "2");
        assert_eq!(one("2026-11-23 + 27"), "2026-12-20, a Sunday");
        assert_eq!(one("2026-03-01 - 1"), "2026-02-28, a Saturday");
        assert_eq!(one("weekday(2026-11-23 + 4 * 7)"), "Monday");
    }
}
