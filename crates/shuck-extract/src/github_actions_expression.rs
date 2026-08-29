//! Adapter from `github-actions-expressions` into Shuck's owned AST.

use github_actions_expressions::{
    Error, Expr, SpannedExpr,
    call::{Call, Function},
    literal::Literal,
    op::{BinExpr, BinOp, UnOp},
};
use shuck_ast::{
    GitHubExpressionBinaryOperator, GitHubExpressionDiagnostic, GitHubExpressionKind,
    GitHubExpressionLiteral, GitHubExpressionNode, GitHubExpressionParse,
    GitHubExpressionUnaryOperator, Name, TextRange, TextSize,
};

pub(crate) fn parse(source: &str) -> GitHubExpressionParse {
    match Expr::parse(source) {
        Ok(expression) => GitHubExpressionParse::Parsed(lower_expression(source, &expression)),
        Err(error) => GitHubExpressionParse::Invalid(lower_error(source, &error)),
    }
}

fn lower_expression(source: &str, expression: &SpannedExpr<'_>) -> GitHubExpressionNode {
    let original_range = text_range(expression.origin.span.start, expression.origin.span.end);
    let (range, groups) = peel_group_ranges(source, original_range);
    let kind = match &expression.inner {
        Expr::Literal(literal) => GitHubExpressionKind::Literal(lower_literal(literal)),
        Expr::Star => GitHubExpressionKind::Star,
        Expr::Call(Call { func, args }) => GitHubExpressionKind::Call {
            function: Name::new(function_name(*func)),
            arguments: args
                .iter()
                .map(|argument| lower_expression(source, argument))
                .collect(),
        },
        Expr::Identifier(identifier) => {
            GitHubExpressionKind::Identifier(Name::new(identifier.as_str()))
        }
        Expr::Index(index) => {
            GitHubExpressionKind::Index(Box::new(lower_expression(source, index)))
        }
        Expr::Context(context) => GitHubExpressionKind::Context(
            context
                .parts
                .iter()
                .map(|part| lower_expression(source, part))
                .collect(),
        ),
        Expr::BinExpr(BinExpr { lhs, op, rhs }) => {
            let left = Box::new(lower_expression(source, lhs));
            let right = Box::new(lower_expression(source, rhs));
            GitHubExpressionKind::Binary {
                operator_range: find_operator_range(
                    source,
                    usize::from(left.range.end()),
                    usize::from(right.range.start()),
                    binary_operator_text(op),
                ),
                left,
                operator: lower_binary_operator(op),
                right,
            }
        }
        Expr::UnExpr { op, expr } => {
            let expression = Box::new(lower_expression(source, expr));
            GitHubExpressionKind::Unary {
                operator: lower_unary_operator(op),
                operator_range: find_operator_range(
                    source,
                    usize::from(range.start()),
                    usize::from(expression.range.start()),
                    unary_operator_text(op),
                ),
                expression,
            }
        }
    };

    let mut node = GitHubExpressionNode { range, kind };
    for group_range in groups.into_iter().rev() {
        node = GitHubExpressionNode {
            range: group_range,
            kind: GitHubExpressionKind::Grouped {
                expression: Box::new(node),
            },
        };
    }
    node
}

fn peel_group_ranges(source: &str, range: TextRange) -> (TextRange, Vec<TextRange>) {
    let mut current = range;
    let mut groups = Vec::new();
    while let Some(inner) = outer_group_inner_range(source, current) {
        groups.push(current);
        current = inner;
    }
    (current, groups)
}

fn outer_group_inner_range(source: &str, range: TextRange) -> Option<TextRange> {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    let slice = source.get(start..end)?;
    let leading = slice.len() - slice.trim_start().len();
    let trailing = slice.len() - slice.trim_end().len();
    let trimmed_start = start + leading;
    let trimmed_end = end - trailing;
    let trimmed = source.get(trimmed_start..trimmed_end)?;
    if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
        return None;
    }

    let bytes = source.as_bytes();
    let mut index = trimmed_start;
    let mut depth = 0usize;
    let mut in_string = false;
    while index < trimmed_end {
        match bytes[index] {
            b'\'' if in_string => {
                if index + 1 < trimmed_end && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                in_string = false;
            }
            b'\'' => in_string = true,
            b'(' if !in_string => depth += 1,
            b')' if !in_string => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && index + 1 != trimmed_end {
                    return None;
                }
            }
            _ => {}
        }
        index += 1;
    }
    if depth != 0 || in_string {
        return None;
    }

    let inner_start = trimmed_start + 1;
    let inner_end = trimmed_end - 1;
    let inner = source.get(inner_start..inner_end)?;
    let leading = inner.len() - inner.trim_start().len();
    let trailing = inner.len() - inner.trim_end().len();
    Some(text_range(inner_start + leading, inner_end - trailing))
}

fn find_operator_range(source: &str, start: usize, end: usize, operator: &str) -> TextRange {
    let Some(between) = source.get(start..end) else {
        return text_range(start, start);
    };
    between.find(operator).map_or_else(
        || text_range(start, start),
        |relative| text_range(start + relative, start + relative + operator.len()),
    )
}

fn lower_literal(literal: &Literal<'_>) -> GitHubExpressionLiteral {
    match literal {
        Literal::Number(number) => GitHubExpressionLiteral::Number(Name::new(number_text(*number))),
        Literal::String(string) => GitHubExpressionLiteral::String(Name::new(string)),
        Literal::Boolean(boolean) => GitHubExpressionLiteral::Boolean(*boolean),
        Literal::Null => GitHubExpressionLiteral::Null,
    }
}

fn number_text(number: f64) -> String {
    if number.is_nan() {
        "NaN".to_owned()
    } else if number == f64::INFINITY {
        "Infinity".to_owned()
    } else if number == f64::NEG_INFINITY {
        "-Infinity".to_owned()
    } else {
        number.to_string()
    }
}

fn function_name(function: Function) -> &'static str {
    match function {
        Function::Contains => "contains",
        Function::StartsWith => "startswith",
        Function::EndsWith => "endswith",
        Function::Format => "format",
        Function::Join => "join",
        Function::ToJSON => "tojson",
        Function::FromJSON => "fromjson",
        Function::HashFiles => "hashfiles",
        Function::Case => "case",
        Function::Success => "success",
        Function::Always => "always",
        Function::Cancelled => "cancelled",
        Function::Failure => "failure",
    }
}

fn lower_binary_operator(operator: &BinOp) -> GitHubExpressionBinaryOperator {
    match operator {
        BinOp::And => GitHubExpressionBinaryOperator::And,
        BinOp::Or => GitHubExpressionBinaryOperator::Or,
        BinOp::Eq => GitHubExpressionBinaryOperator::Equal,
        BinOp::Neq => GitHubExpressionBinaryOperator::NotEqual,
        BinOp::Gt => GitHubExpressionBinaryOperator::GreaterThan,
        BinOp::Ge => GitHubExpressionBinaryOperator::GreaterThanOrEqual,
        BinOp::Lt => GitHubExpressionBinaryOperator::LessThan,
        BinOp::Le => GitHubExpressionBinaryOperator::LessThanOrEqual,
    }
}

fn binary_operator_text(operator: &BinOp) -> &'static str {
    match operator {
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Eq => "==",
        BinOp::Neq => "!=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
    }
}

fn lower_unary_operator(operator: &UnOp) -> GitHubExpressionUnaryOperator {
    match operator {
        UnOp::Not => GitHubExpressionUnaryOperator::Not,
    }
}

fn unary_operator_text(operator: &UnOp) -> &'static str {
    match operator {
        UnOp::Not => "!",
    }
}

fn lower_error(source: &str, error: &Error) -> GitHubExpressionDiagnostic {
    let (message, offset) = match error {
        Error::Syntax(error) => (error.to_string(), error.offset),
        Error::Call(error) => (error.to_string(), call_error_offset(source, error)),
    };

    GitHubExpressionDiagnostic {
        message,
        range: text_range(offset, offset),
    }
}

fn call_error_offset(source: &str, error: &github_actions_expressions::call::Error) -> usize {
    match error {
        github_actions_expressions::call::Error::UnknownFunction(function) => {
            find_ascii_case_insensitive(source, function).unwrap_or(0)
        }
        github_actions_expressions::call::Error::Arity(function, _) => {
            find_ascii_case_insensitive(source, function_name(*function)).unwrap_or(0)
        }
    }
}

fn find_ascii_case_insensitive(source: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    source
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn text_range(start: usize, end: usize) -> TextRange {
    TextRange::new(text_size(start), text_size(end))
}

fn text_size(offset: usize) -> TextSize {
    TextSize::new(u32::try_from(offset).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use shuck_ast::{GitHubExpressionKind, GitHubExpressionLiteral, GitHubExpressionParse};

    use super::parse;

    #[test]
    fn lowers_nested_expression_with_exact_ranges() {
        let source = "github.ref == 'main' && format('{0}', matrix.os)";
        let GitHubExpressionParse::Parsed(root) = parse(source) else {
            panic!("expected a parsed expression");
        };

        assert_eq!(root.source(source), source);
        let GitHubExpressionKind::Binary { left, right, .. } = &root.kind else {
            panic!("expected a logical binary expression");
        };
        assert_eq!(left.source(source), "github.ref == 'main'");
        assert_eq!(right.source(source), "format('{0}', matrix.os)");

        let GitHubExpressionKind::Binary {
            left: context,
            right: literal,
            ..
        } = &left.kind
        else {
            panic!("expected an equality expression");
        };
        assert_eq!(context.source(source), "github.ref");
        assert_eq!(literal.source(source), "'main'");
        assert!(matches!(
            literal.kind,
            GitHubExpressionKind::Literal(GitHubExpressionLiteral::String(ref value))
                if value == "main"
        ));

        let GitHubExpressionKind::Call {
            function,
            arguments,
        } = &right.kind
        else {
            panic!("expected a function call");
        };
        assert_eq!(function, "format");
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[1].source(source), "matrix.os");
    }

    #[test]
    fn lowers_access_chains_filters_and_computed_indices() {
        let source = "fromJSON(needs.build.outputs.data).items[github.run_id].*.name";
        let GitHubExpressionParse::Parsed(root) = parse(source) else {
            panic!("expected a parsed expression");
        };

        let GitHubExpressionKind::Context(parts) = &root.kind else {
            panic!("expected a context chain");
        };
        assert_eq!(parts.len(), 5);
        assert_eq!(
            parts[0].source(source),
            "fromJSON(needs.build.outputs.data)"
        );
        assert_eq!(parts[1].source(source), "items");
        assert_eq!(parts[2].source(source), "[github.run_id]");
        assert!(matches!(parts[3].kind, GitHubExpressionKind::Star));
        assert_eq!(parts[4].source(source), "name");
    }

    #[test]
    fn reconstructs_balanced_group_nodes() {
        let source = "((github.ref))";
        let GitHubExpressionParse::Parsed(root) = parse(source) else {
            panic!("expected a parsed expression");
        };

        assert_eq!(root.source(source), source);
        let GitHubExpressionKind::Grouped { expression } = &root.kind else {
            panic!("expected outer grouping");
        };
        assert_eq!(expression.source(source), "(github.ref)");
        let GitHubExpressionKind::Grouped { expression } = &expression.kind else {
            panic!("expected inner grouping");
        };
        assert_eq!(expression.source(source), "github.ref");
        assert!(matches!(expression.kind, GitHubExpressionKind::Context(_)));
    }

    #[test]
    fn records_binary_and_unary_operator_ranges() {
        let source = "!failure() || github.ref == 'main'";
        let GitHubExpressionParse::Parsed(root) = parse(source) else {
            panic!("expected a parsed expression");
        };
        let GitHubExpressionKind::Binary {
            left,
            operator_range,
            right,
            ..
        } = &root.kind
        else {
            panic!("expected binary expression");
        };
        assert_eq!(operator_range.slice(source), "||");
        let GitHubExpressionKind::Unary { operator_range, .. } = &left.kind else {
            panic!("expected unary expression");
        };
        assert_eq!(operator_range.slice(source), "!");
        let GitHubExpressionKind::Binary { operator_range, .. } = &right.kind else {
            panic!("expected equality expression");
        };
        assert_eq!(operator_range.slice(source), "==");
    }

    #[test]
    fn reports_syntax_and_call_errors_at_expression_local_offsets() {
        let GitHubExpressionParse::Invalid(syntax) = parse("github.") else {
            panic!("expected invalid syntax");
        };
        assert_eq!(usize::from(syntax.range.start()), 7);
        assert!(syntax.message.contains("identifier"));

        let GitHubExpressionParse::Invalid(call) = parse("mystery(github.ref)") else {
            panic!("expected an unknown function error");
        };
        assert_eq!(usize::from(call.range.start()), 0);
        assert!(call.message.contains("mystery"));
    }
}
