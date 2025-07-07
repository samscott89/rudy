//! Expression parsing for debugger evaluation
//!
//! This module provides parsing for Rust-like expressions for use in
//! debugger evaluation. Supports field access, array indexing, dereferencing,
//! and other common expression forms.

use std::fmt;

use anyhow::{Result, anyhow};
use itertools::Itertools;

/// Represents a parsed expression
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// Simple variable reference (e.g., `foo`)
    Variable(String),

    /// Path expression (e.g., `std::vec::Vec`, `lldb_demo::User`)
    Path(Vec<String>),

    /// Generic type (e.g., `Vec<String>`, `HashMap<String, u32>`)
    Generic { base: String, args: Vec<String> },

    /// Field access (e.g., `foo.bar`, `self.field`)
    FieldAccess {
        base: Box<Expression>,
        field: String,
    },

    /// Array/slice indexing (e.g., `arr[5]`, `slice[idx]`)
    Index {
        base: Box<Expression>,
        index: Box<Expression>,
    },

    /// Pointer dereferencing (e.g., `*ptr`, `**ptr_ptr`)
    Deref(Box<Expression>),

    /// Address-of operator (e.g., `&var`, `&mut var`)
    AddressOf {
        mutable: bool,
        expr: Box<Expression>,
    },

    /// Literal number (e.g., `42`, `0xff`)
    NumberLiteral(u64),

    /// String literal (e.g., `"hello"`, `"created"`)
    StringLiteral(String),

    /// Parenthesized expression (e.g., `(foo)`)
    Parenthesized(Box<Expression>),

    /// Method call (e.g., `foo.bar()`, `vec.len()`)
    MethodCall {
        base: Box<Expression>,
        method: String,
        args: Vec<Expression>,
    },

    /// Function call (e.g., `foo()`, `bar(1, 2)`)
    FunctionCall {
        function: String,
        args: Vec<Expression>,
    },
}

impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expression::Variable(name) => write!(f, "{name}"),
            Expression::Path(segments) => write!(f, "{}", segments.join("::")),
            Expression::Generic { base, args } => write!(f, "{}<{}>", base, args.join(", ")),
            Expression::FieldAccess { base, field } => write!(f, "{base}.{field}"),
            Expression::Index { base, index } => write!(f, "{base}[{index}]"),
            Expression::Deref(expr) => write!(f, "*{expr}"),
            Expression::AddressOf { mutable, expr } => {
                if *mutable {
                    write!(f, "&mut {expr}")
                } else {
                    write!(f, "&{expr}")
                }
            }
            Expression::NumberLiteral(value) => write!(f, "{value}"),
            Expression::StringLiteral(value) => write!(f, "\"{value}\""),
            Expression::Parenthesized(expr) => write!(f, "({expr})"),
            Expression::MethodCall { base, method, args } => {
                write!(f, "{base}.{method}({})", args.iter().join(", "))
            }
            Expression::FunctionCall { function, args } => {
                write!(f, "{function}({})", args.iter().join(", "))
            }
        }
    }
}

/// Simple tokenizer for expressions
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Identifier(String),
    Number(u64),
    String(String),
    Dot,
    Star,
    Ampersand,
    /// '['
    LeftBracket,
    /// ']'
    RightBracket,
    /// '('
    LeftParen,
    /// ')'
    RightParen,
    /// '<'
    LeftAngle,
    /// '>'
    RightAngle,
    /// '::'
    PathSeparator,
    /// ','
    Comma,
    Mut,
    Eof,
}

struct Tokenizer<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    fn current_char(&self) -> Option<char> {
        self.input.chars().nth(self.position)
    }

    fn advance(&mut self) {
        if self.position < self.input.len() {
            self.position += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char() {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_identifier(&mut self) -> String {
        let start = self.position;
        while let Some(ch) = self.current_char() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }
        self.input[start..self.position].to_string()
    }

    fn read_number(&mut self) -> Result<u64> {
        let start = self.position;

        // Check for hex prefix
        if self.input[self.position..].starts_with("0x")
            || self.input[self.position..].starts_with("0X")
        {
            self.advance(); // skip '0'
            self.advance(); // skip 'x'
            while let Some(ch) = self.current_char() {
                if ch.is_ascii_hexdigit() {
                    self.advance();
                } else {
                    break;
                }
            }
            let hex_str = &self.input[start + 2..self.position];
            u64::from_str_radix(hex_str, 16).map_err(|e| anyhow!("Invalid hex number: {}", e))
        } else {
            // Regular decimal number
            while let Some(ch) = self.current_char() {
                if ch.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
            self.input[start..self.position]
                .parse()
                .map_err(|e| anyhow!("Invalid number: {}", e))
        }
    }

    fn read_string(&mut self) -> Result<String> {
        // Skip opening quote
        self.advance();
        let start = self.position;

        while let Some(ch) = self.current_char() {
            if ch == '"' {
                // Found closing quote
                let content = self.input[start..self.position].to_string();
                self.advance(); // Skip closing quote
                return Ok(content);
            } else if ch == '\\' {
                // Handle escape sequences (basic support)
                self.advance(); // Skip backslash
                if self.current_char().is_some() {
                    self.advance(); // Skip escaped character
                }
            } else {
                self.advance();
            }
        }

        Err(anyhow!("Unterminated string literal"))
    }

    fn next_token(&mut self) -> Result<Token> {
        self.skip_whitespace();

        match self.current_char() {
            None => Ok(Token::Eof),
            Some('.') => {
                self.advance();
                Ok(Token::Dot)
            }
            Some('*') => {
                self.advance();
                Ok(Token::Star)
            }
            Some('&') => {
                self.advance();
                Ok(Token::Ampersand)
            }
            Some('[') => {
                self.advance();
                Ok(Token::LeftBracket)
            }
            Some(']') => {
                self.advance();
                Ok(Token::RightBracket)
            }
            Some('(') => {
                self.advance();
                Ok(Token::LeftParen)
            }
            Some(')') => {
                self.advance();
                Ok(Token::RightParen)
            }
            Some('<') => {
                self.advance();
                Ok(Token::LeftAngle)
            }
            Some('>') => {
                self.advance();
                Ok(Token::RightAngle)
            }
            Some(':') => {
                self.advance();
                if self.current_char() == Some(':') {
                    self.advance();
                    Ok(Token::PathSeparator)
                } else {
                    Err(anyhow!("Expected '::' but found single ':'"))
                }
            }
            Some(',') => {
                self.advance();
                Ok(Token::Comma)
            }
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {
                let ident = self.read_identifier();
                if ident == "mut" {
                    Ok(Token::Mut)
                } else {
                    Ok(Token::Identifier(ident))
                }
            }
            Some(ch) if ch.is_ascii_digit() => {
                let number = self.read_number()?;
                Ok(Token::Number(number))
            }
            Some('"') => {
                let string = self.read_string()?;
                Ok(Token::String(string))
            }
            Some(ch) => Err(anyhow!("Unexpected character: '{}'", ch)),
        }
    }
}

/// Simple recursive descent parser
struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn new(input: &str) -> Result<Self> {
        let mut tokenizer = Tokenizer::new(input);
        let mut tokens = Vec::new();

        loop {
            let token = tokenizer.next_token()?;
            let is_eof = token == Token::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }

        Ok(Self {
            tokens,
            position: 0,
        })
    }

    fn current_token(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: Token) -> Result<()> {
        if std::mem::discriminant(self.current_token()) == std::mem::discriminant(&expected) {
            self.advance();
            Ok(())
        } else {
            Err(anyhow!(
                "Expected {:?}, found {:?}",
                expected,
                self.current_token()
            ))
        }
    }

    pub fn parse(&mut self) -> Result<Expression> {
        self.parse_expression()
    }

    fn parse_expression(&mut self) -> Result<Expression> {
        self.parse_unary()
    }

    fn parse_unary(&mut self) -> Result<Expression> {
        match self.current_token() {
            Token::Star => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expression::Deref(Box::new(expr)))
            }
            Token::Ampersand => {
                self.advance();
                let mutable = matches!(self.current_token(), Token::Mut);
                if mutable {
                    self.advance();
                }
                let expr = self.parse_unary()?;
                Ok(Expression::AddressOf {
                    mutable,
                    expr: Box::new(expr),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expression> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.current_token() {
                Token::Dot => {
                    self.advance();
                    if let Token::Identifier(field) = self.current_token() {
                        let field = field.clone();
                        self.advance();

                        // Check if this is a method call
                        if matches!(self.current_token(), Token::LeftParen) {
                            self.advance(); // consume '('
                            let args = self.parse_arguments()?;
                            self.expect(Token::RightParen)?;
                            expr = Expression::MethodCall {
                                base: Box::new(expr),
                                method: field,
                                args,
                            };
                        } else {
                            expr = Expression::FieldAccess {
                                base: Box::new(expr),
                                field,
                            };
                        }
                    } else {
                        return Err(anyhow!("Expected field name after '.'"));
                    }
                }
                Token::LeftBracket => {
                    self.advance();
                    let index = self.parse_expression()?;
                    self.expect(Token::RightBracket)?;
                    expr = Expression::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expression> {
        match self.current_token() {
            Token::Identifier(name) => {
                let first_segment = name.clone();
                self.advance();

                // Check if this is a path (e.g., std::vec::Vec)
                let mut segments = vec![first_segment.clone()];
                while matches!(self.current_token(), Token::PathSeparator) {
                    self.advance(); // consume '::'
                    if let Token::Identifier(segment) = self.current_token() {
                        segments.push(segment.clone());
                        self.advance();
                    } else {
                        return Err(anyhow!("Expected identifier after '::'"));
                    }
                }

                // Check if this is a generic type (e.g., Vec<String>)
                if matches!(self.current_token(), Token::LeftAngle) {
                    self.advance(); // consume '<'
                    let type_args = self.parse_type_arguments()?;
                    self.expect(Token::RightAngle)?;

                    // For generics, we use the full path as the base
                    let base = segments.join("::");
                    return Ok(Expression::Generic {
                        base,
                        args: type_args,
                    });
                }

                // Check if this is a function call
                if matches!(self.current_token(), Token::LeftParen) {
                    self.advance(); // consume '('
                    let args = self.parse_arguments()?;
                    self.expect(Token::RightParen)?;

                    // For function calls, use the full path as the function name
                    let function = segments.join("::");
                    return Ok(Expression::FunctionCall { function, args });
                }

                // Return as path if multiple segments, otherwise as variable
                if segments.len() > 1 {
                    Ok(Expression::Path(segments))
                } else {
                    Ok(Expression::Variable(first_segment))
                }
            }
            Token::Number(value) => {
                let value = *value;
                self.advance();
                Ok(Expression::NumberLiteral(value))
            }
            Token::String(value) => {
                let value = value.clone();
                self.advance();
                Ok(Expression::StringLiteral(value))
            }
            Token::LeftParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(Token::RightParen)?;
                Ok(Expression::Parenthesized(Box::new(expr)))
            }
            _ => Err(anyhow!(
                "Expected identifier, number, string, or '(', found {:?}",
                self.current_token()
            )),
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<Expression>> {
        let mut args = Vec::new();

        // Handle empty argument list
        if matches!(self.current_token(), Token::RightParen) {
            return Ok(args);
        }

        // Parse first argument
        args.push(self.parse_expression()?);

        // Parse remaining arguments
        while matches!(self.current_token(), Token::Comma) {
            self.advance(); // consume ','
            args.push(self.parse_expression()?);
        }

        Ok(args)
    }

    fn parse_type_arguments(&mut self) -> Result<Vec<String>> {
        let mut args = Vec::new();

        // Handle empty type argument list
        if matches!(self.current_token(), Token::RightAngle) {
            return Ok(args);
        }

        // Parse first type argument
        args.push(self.parse_type_name()?);

        // Parse remaining type arguments
        while matches!(self.current_token(), Token::Comma) {
            self.advance(); // consume ','
            args.push(self.parse_type_name()?);
        }

        Ok(args)
    }

    fn parse_type_name(&mut self) -> Result<String> {
        let mut segments = Vec::new();

        // Parse first segment
        if let Token::Identifier(name) = self.current_token() {
            segments.push(name.clone());
            self.advance();
        } else {
            return Err(anyhow!("Expected type name"));
        }

        // Parse additional path segments
        while matches!(self.current_token(), Token::PathSeparator) {
            self.advance(); // consume '::'
            if let Token::Identifier(segment) = self.current_token() {
                segments.push(segment.clone());
                self.advance();
            } else {
                return Err(anyhow!("Expected identifier after '::'"));
            }
        }

        let mut type_name = segments.join("::");

        // Check for nested generics
        if matches!(self.current_token(), Token::LeftAngle) {
            self.advance(); // consume '<'
            let nested_args = self.parse_type_arguments()?;
            self.expect(Token::RightAngle)?;
            type_name.push('<');
            type_name.push_str(&nested_args.join(", "));
            type_name.push('>');
        }

        Ok(type_name)
    }
}

/// Parse a string into an Expression
pub fn parse_expression(input: &str) -> Result<Expression> {
    let mut parser = Parser::new(input)?;
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn parse(s: &str) -> Expression {
        match parse_expression(s) {
            Ok(expr) => expr,
            Err(e) => panic!("Failed to parse expression '{s}': {e}"),
        }
    }

    #[test]
    fn test_variable() {
        let expr = parse("foo");
        assert_eq!(expr, Expression::Variable("foo".to_string()));
    }

    #[test]
    fn test_number_literal() {
        let expr = parse("42");
        assert_eq!(expr, Expression::NumberLiteral(42));

        let expr = parse("0xff");
        assert_eq!(expr, Expression::NumberLiteral(0xff));
    }

    #[test]
    fn test_string_literal() {
        let expr = parse(r#""hello""#);
        assert_eq!(expr, Expression::StringLiteral("hello".to_string()));

        let expr = parse(r#""created""#);
        assert_eq!(expr, Expression::StringLiteral("created".to_string()));
    }

    #[test]
    fn test_field_access() {
        let expr = parse("foo.bar");
        assert_eq!(
            expr,
            Expression::FieldAccess {
                base: Box::new(Expression::Variable("foo".to_string())),
                field: "bar".to_string(),
            }
        );
    }

    #[test]
    fn test_chained_field_access() {
        let expr = parse("foo.bar.baz");
        assert_eq!(
            expr,
            Expression::FieldAccess {
                base: Box::new(Expression::FieldAccess {
                    base: Box::new(Expression::Variable("foo".to_string())),
                    field: "bar".to_string(),
                }),
                field: "baz".to_string(),
            }
        );
    }

    #[test]
    fn test_index_access() {
        let expr = parse("arr[0]");
        assert_eq!(
            expr,
            Expression::Index {
                base: Box::new(Expression::Variable("arr".to_string())),
                index: Box::new(Expression::NumberLiteral(0)),
            }
        );

        // Test string indexing
        let expr = parse(r#"map["key"]"#);
        assert_eq!(
            expr,
            Expression::Index {
                base: Box::new(Expression::Variable("map".to_string())),
                index: Box::new(Expression::StringLiteral("key".to_string())),
            }
        );
    }

    #[test]
    fn test_deref() {
        let expr = parse("*ptr");
        assert_eq!(
            expr,
            Expression::Deref(Box::new(Expression::Variable("ptr".to_string())))
        );
    }

    #[test]
    fn test_address_of() {
        let expr = parse("&var");
        assert_eq!(
            expr,
            Expression::AddressOf {
                mutable: false,
                expr: Box::new(Expression::Variable("var".to_string())),
            }
        );

        let expr = parse("&mut var");
        assert_eq!(
            expr,
            Expression::AddressOf {
                mutable: true,
                expr: Box::new(Expression::Variable("var".to_string())),
            }
        );
    }

    #[test]
    fn test_parenthesized() {
        let expr = parse("(foo)");
        assert_eq!(
            expr,
            Expression::Parenthesized(Box::new(Expression::Variable("foo".to_string())))
        );
    }

    #[test]
    fn test_complex_expressions() {
        // Test field access with indexing: obj.field[0]
        let expr = parse("obj.field[0]");
        assert_eq!(
            expr,
            Expression::Index {
                base: Box::new(Expression::FieldAccess {
                    base: Box::new(Expression::Variable("obj".to_string())),
                    field: "field".to_string(),
                }),
                index: Box::new(Expression::NumberLiteral(0)),
            }
        );

        // Test dereferencing field access: *obj.ptr
        let expr = parse("*obj.ptr");
        assert_eq!(
            expr,
            Expression::Deref(Box::new(Expression::FieldAccess {
                base: Box::new(Expression::Variable("obj".to_string())),
                field: "ptr".to_string(),
            }))
        );
    }

    #[test]
    fn test_display_formatting() {
        assert_eq!(parse("foo").to_string(), "foo");
        assert_eq!(parse("42").to_string(), "42");
        assert_eq!(parse(r#""hello""#).to_string(), r#""hello""#);
        assert_eq!(parse("foo.bar").to_string(), "foo.bar");
        assert_eq!(parse("arr[0]").to_string(), "arr[0]");
        assert_eq!(parse(r#"map["key"]"#).to_string(), r#"map["key"]"#);
        assert_eq!(parse("*ptr").to_string(), "*ptr");
        assert_eq!(parse("&var").to_string(), "&var");
        assert_eq!(parse("&mut var").to_string(), "&mut var");
        assert_eq!(parse("(foo)").to_string(), "(foo)");
    }

    #[test]
    fn test_method_call() {
        // No arguments
        let expr = parse("vec.len()");
        assert_eq!(
            expr,
            Expression::MethodCall {
                base: Box::new(Expression::Variable("vec".to_string())),
                method: "len".to_string(),
                args: vec![],
            }
        );

        // With arguments
        let expr = parse("vec.push(42)");
        assert_eq!(
            expr,
            Expression::MethodCall {
                base: Box::new(Expression::Variable("vec".to_string())),
                method: "push".to_string(),
                args: vec![Expression::NumberLiteral(42)],
            }
        );

        // Multiple arguments
        let expr = parse(r#"map.insert("key", 42)"#);
        assert_eq!(
            expr,
            Expression::MethodCall {
                base: Box::new(Expression::Variable("map".to_string())),
                method: "insert".to_string(),
                args: vec![
                    Expression::StringLiteral("key".to_string()),
                    Expression::NumberLiteral(42)
                ],
            }
        );

        // Chained method calls
        let expr = parse("vec.iter().count()");
        assert_eq!(
            expr,
            Expression::MethodCall {
                base: Box::new(Expression::MethodCall {
                    base: Box::new(Expression::Variable("vec".to_string())),
                    method: "iter".to_string(),
                    args: vec![],
                }),
                method: "count".to_string(),
                args: vec![],
            }
        );
    }

    #[test]
    fn test_function_call() {
        // No arguments
        let expr = parse("foo()");
        assert_eq!(
            expr,
            Expression::FunctionCall {
                function: "foo".to_string(),
                args: vec![],
            }
        );

        // With arguments
        let expr = parse("bar(1, 2)");
        assert_eq!(
            expr,
            Expression::FunctionCall {
                function: "bar".to_string(),
                args: vec![Expression::NumberLiteral(1), Expression::NumberLiteral(2)],
            }
        );
    }

    #[test]
    fn test_path_expressions() {
        // Simple path
        let expr = parse("std::vec::Vec");
        assert_eq!(
            expr,
            Expression::Path(vec![
                "std".to_string(),
                "vec".to_string(),
                "Vec".to_string()
            ])
        );

        // Path function call
        let expr = parse("lldb_demo::User::new(1, \"foo\", \"bar\")");
        assert_eq!(
            expr,
            Expression::FunctionCall {
                function: "lldb_demo::User::new".to_string(),
                args: vec![
                    Expression::NumberLiteral(1),
                    Expression::StringLiteral("foo".to_string()),
                    Expression::StringLiteral("bar".to_string())
                ],
            }
        );
    }

    #[test]
    fn test_generic_types() {
        // Simple generic
        let expr = parse("Vec<u64>");
        assert_eq!(
            expr,
            Expression::Generic {
                base: "Vec".to_string(),
                args: vec!["u64".to_string()],
            }
        );

        // Generic with multiple type arguments
        let expr = parse("HashMap<String, u32>");
        assert_eq!(
            expr,
            Expression::Generic {
                base: "HashMap".to_string(),
                args: vec!["String".to_string(), "u32".to_string()],
            }
        );

        // Nested generics
        let expr = parse("Vec<Option<String>>");
        assert_eq!(
            expr,
            Expression::Generic {
                base: "Vec".to_string(),
                args: vec!["Option<String>".to_string()],
            }
        );

        // Path with generics
        let expr = parse("std::collections::HashMap<String, u32>");
        assert_eq!(
            expr,
            Expression::Generic {
                base: "std::collections::HashMap".to_string(),
                args: vec!["String".to_string(), "u32".to_string()],
            }
        );
    }

    #[test]
    fn test_method_call_display() {
        assert_eq!(parse("vec.len()").to_string(), "vec.len()");
        assert_eq!(parse("vec.push(42)").to_string(), "vec.push(42)");
        assert_eq!(
            parse(r#"map.insert("key", 42)"#).to_string(),
            r#"map.insert("key", 42)"#
        );
        assert_eq!(parse("foo()").to_string(), "foo()");
        assert_eq!(parse("bar(1, 2)").to_string(), "bar(1, 2)");
    }

    #[test]
    fn test_path_and_generic_display() {
        // Path expressions
        assert_eq!(parse("std::vec::Vec").to_string(), "std::vec::Vec");
        assert_eq!(
            parse("lldb_demo::User::new(1, \"foo\", \"bar\")").to_string(),
            r#"lldb_demo::User::new(1, "foo", "bar")"#
        );

        // Generic expressions
        assert_eq!(parse("Vec<u64>").to_string(), "Vec<u64>");
        assert_eq!(
            parse("HashMap<String, u32>").to_string(),
            "HashMap<String, u32>"
        );
        assert_eq!(
            parse("Vec<Option<String>>").to_string(),
            "Vec<Option<String>>"
        );
        assert_eq!(
            parse("std::collections::HashMap<String, u32>").to_string(),
            "std::collections::HashMap<String, u32>"
        );
    }
}
