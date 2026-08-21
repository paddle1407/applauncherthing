#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Atom(i64),
    Plus,
    Minus,
    Slash,
    Star,
    LParen,
    RParen,
    Invalid,
    Eof,
}

#[derive(Debug)]
pub struct Lexer {
    tokens: Vec<Token>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let mut tokens = Vec::new();
        let mut chars = input.chars().peekable();

        while let Some(&ch) = chars.peek() {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    chars.next();
                }
                '+' => {
                    tokens.push(Token::Plus);
                    chars.next();
                }
                '-' => {
                    tokens.push(Token::Minus);
                    chars.next();
                }
                '*' => {
                    tokens.push(Token::Star);
                    chars.next();
                }
                '/' => {
                    tokens.push(Token::Slash);
                    chars.next();
                }
                '(' => {
                    tokens.push(Token::LParen);
                    chars.next();
                }
                ')' => {
                    tokens.push(Token::RParen);
                    chars.next();
                }
                '0'..='9' => {
                    let mut num = String::new();
                    while let Some(&digit) = chars.peek() {
                        if digit.is_ascii_digit() {
                            num.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    match num.parse::<i64>() {
                        Ok(val) => tokens.push(Token::Atom(val)),
                        Err(_) => tokens.push(Token::Invalid),
                    }
                }
                _ => {
                    tokens.push(Token::Invalid);
                    chars.next();
                }
            }
        }

        tokens.reverse();
        Lexer { tokens }
    }

    pub fn peek(&self) -> Token {
        self.tokens.last().copied().unwrap_or(Token::Eof)
    }

    pub fn next(&mut self) -> Token {
        self.tokens.pop().unwrap_or(Token::Eof)
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Atom(i64),
    UnOp(Token, Box<Expr>),
    BinOp(Token, Box<Expr>, Box<Expr>),
}

fn prefix_binding_power(operator: Token) -> Option<((), u8)> {
    match operator {
        Token::Minus | Token::Plus => Some(((), 5)),
        _ => None,
    }
}

fn infix_binding_power(operator: Token) -> Option<(u8, u8)> {
    match operator {
        Token::Plus | Token::Minus => Some((1, 2)),
        Token::Star | Token::Slash => Some((3, 4)),
        _ => None,
    }
}

pub fn parse_expr(lexer: &mut Lexer, min_bp: u8) -> Result<Expr, String> {
    let token = lexer.next();

    let mut left = match token {
        Token::Atom(val) => Expr::Atom(val),
        Token::LParen => {
            let expr = parse_expr(lexer, 0)?;
            if lexer.next() != Token::RParen {
                return Err("Missing closing parenthesis ')'".to_string());
            }
            expr
        }
        op @ (Token::Minus | Token::Plus) => {
            let ((), r_bp) = prefix_binding_power(op).unwrap();
            let rhs = parse_expr(lexer, r_bp)?;
            Expr::UnOp(op, Box::new(rhs))
        }
        t => return Err(format!("Unexpected token at start of expression: {:?}", t)),
    };

    loop {
        let op = lexer.peek();
        if op == Token::Eof || op == Token::RParen {
            break;
        }

        if let Some((l_bp, r_bp)) = infix_binding_power(op) {
            if l_bp < min_bp {
                break;
            }

            lexer.next();
            let rhs = parse_expr(lexer, r_bp)?;
            left = Expr::BinOp(op, Box::new(left), Box::new(rhs));
            continue;
        }

        break;
    }

    Ok(left)
}

pub fn eval(expr: &Expr) -> Result<i64, String> {
    match expr {
        Expr::Atom(val) => Ok(*val),
        Expr::UnOp(Token::Minus, rhs) => eval(rhs)?
            .checked_neg()
            .ok_or_else(|| "integer overflow".to_string()),
        Expr::UnOp(Token::Plus, rhs) => eval(rhs),
        Expr::BinOp(op, lhs, rhs) => {
            let left_val = eval(lhs)?;
            let right_val = eval(rhs)?;
            match op {
                Token::Plus => left_val
                    .checked_add(right_val)
                    .ok_or_else(|| "integer overflow".to_string()),
                Token::Minus => left_val
                    .checked_sub(right_val)
                    .ok_or_else(|| "integer overflow".to_string()),
                Token::Star => left_val
                    .checked_mul(right_val)
                    .ok_or_else(|| "integer overflow".to_string()),
                Token::Slash => {
                    if right_val == 0 {
                        Err("you cant divide by zero".to_string())
                    } else {
                        left_val
                            .checked_div(right_val)
                            .ok_or_else(|| "integer overflow".to_string())
                    }
                }
                _ => Err("invalid operator".to_string()),
            }
        }
        _ => Err("invalid tree".to_string()),
    }
}

pub fn calculate(input: &str) -> Result<i64, String> {
    let mut lexer = Lexer::new(input);
    let ast = parse_expr(&mut lexer, 0)?;

    if lexer.peek() != Token::Eof {
        return Err("unexpected trailing input".to_string());
    }

    eval(&ast)
}

#[cfg(test)]
mod tests {
    use super::calculate;

    #[test]
    fn rejects_unknown_and_trailing_input() {
        for input in ["2garbage", "2 3", "2$+3", "(1)2"] {
            assert!(calculate(input).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn rejects_integer_overflow() {
        assert!(calculate("9223372036854775807+1").is_err());
        assert!(calculate("3037000500*3037000500").is_err());
        assert!(calculate("-(-9223372036854775807-1)").is_err());
    }

    #[test]
    fn evaluates_valid_expressions() {
        assert_eq!(calculate("2 + 3 * (4 - 1)"), Ok(11));
        assert_eq!(calculate("-7 / 2"), Ok(-3));
    }
}
