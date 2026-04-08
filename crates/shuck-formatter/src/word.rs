use std::fmt::Write as _;

use shuck_ast::{
    ArithmeticAssignOp, ArithmeticBinaryOp, ArithmeticExpansionSyntax, ArithmeticExpr,
    ArithmeticExprNode, ArithmeticLvalue, ArithmeticPostfixOp, ArithmeticUnaryOp,
    BourneParameterExpansion, CommandSubstitutionSyntax, ParameterOp, Pattern, SubscriptSelector,
    VarRef, Word, WordPart,
};
use shuck_format::IndentStyle;
use shuck_format::{FormatResult, text, write};

use crate::FormatNodeRule;
use crate::command::format_stmt_sequence;
use crate::comments::Comments;
use crate::context::ShellFormatContext;
use crate::options::ResolvedShellFormatOptions;
use crate::prelude::ShellFormatter;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatWord;

impl FormatNodeRule<Word> for FormatWord {
    fn fmt(&self, word: &Word, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let rendered = render_word_syntax(
            word,
            formatter.context().source(),
            formatter.context().options(),
        );
        write!(formatter, [text(rendered)])
    }
}

pub(crate) fn render_word_syntax(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    if let Some(rendered) = render_special_word_syntax(word, source, options) {
        return rendered;
    }

    let rendered = word.render_syntax(source);

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_word_source_slice(word, source)
        && should_preserve_raw_syntax(slice, &rendered)
    {
        return slice.to_string();
    }

    rendered
}

fn render_special_word_syntax(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if !word_needs_special_rendering(word) {
        return None;
    }

    render_word_parts(word.parts.as_slice(), source, options).ok()
}

fn word_needs_special_rendering(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| part_needs_special_rendering(&part.kind))
}

fn part_needs_special_rendering(part: &WordPart) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_needs_special_rendering(&part.kind)),
        WordPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_some(),
        WordPart::Parameter(parameter) => parameter.bourne().is_some_and(|syntax| match syntax {
            BourneParameterExpansion::Slice {
                offset_ast,
                length_ast,
                ..
            } => offset_ast.is_some() || length_ast.is_some(),
            _ => false,
        }),
        WordPart::Substring {
            offset_ast,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            offset_ast,
            length_ast,
            ..
        } => offset_ast.is_some() || length_ast.is_some(),
        WordPart::CommandSubstitution { syntax, .. } => {
            *syntax == CommandSubstitutionSyntax::DollarParen
        }
        _ => false,
    }
}

fn render_word_parts(
    parts: &[shuck_ast::WordPartNode],
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Result<String, std::fmt::Error> {
    let mut rendered = String::new();
    for part in parts {
        render_word_part(&mut rendered, &part.kind, part.span, source, options)?;
    }
    Ok(rendered)
}

fn render_word_part(
    rendered: &mut String,
    part: &WordPart,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Result<(), std::fmt::Error> {
    match part {
        WordPart::Literal(text) => rendered.push_str(text.as_str(source, span)),
        WordPart::SingleQuoted { value, dollar } => {
            if *dollar {
                rendered.push('$');
            }
            rendered.push('\'');
            rendered.push_str(value.slice(source));
            rendered.push('\'');
        }
        WordPart::DoubleQuoted { parts, dollar } => {
            if *dollar {
                rendered.push('$');
            }
            rendered.push('"');
            for part in parts {
                match &part.kind {
                    WordPart::Literal(text) => {
                        render_double_quoted_literal(rendered, text.as_str(source, part.span))
                    }
                    other => render_word_part(rendered, other, part.span, source, options)?,
                }
            }
            rendered.push('"');
        }
        WordPart::Variable(name) => {
            std::write!(rendered, "${name}")?;
        }
        WordPart::CommandSubstitution { body, syntax } => {
            if *syntax == CommandSubstitutionSyntax::Backtick {
                rendered.push_str(span.slice(source));
            } else if let Some(raw) = raw_source_slice(span, source) {
                if raw.contains('#') {
                    rendered.push_str(raw);
                } else if let Some(command_substitution) =
                    render_command_substitution(body, source, options, raw.contains('\n'))
                {
                    rendered.push_str(&command_substitution);
                } else {
                    rendered.push_str(raw);
                }
            } else if let Some(command_substitution) =
                render_command_substitution(body, source, options, true)
            {
                rendered.push_str(&command_substitution);
            } else {
                std::write!(rendered, "$({body:?})")?;
            }
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
        } => {
            if let Some(expression_ast) = expression_ast {
                match syntax {
                    ArithmeticExpansionSyntax::DollarParenParen => {
                        rendered.push_str("$((");
                        rendered.push_str(&render_arithmetic_expr(expression_ast, source, options));
                        rendered.push_str("))");
                    }
                    ArithmeticExpansionSyntax::LegacyBracket => {
                        rendered.push_str("$[");
                        rendered.push_str(&render_arithmetic_expr(expression_ast, source, options));
                        rendered.push(']');
                    }
                }
            } else {
                match syntax {
                    ArithmeticExpansionSyntax::DollarParenParen => {
                        std::write!(rendered, "$(({}))", expression.slice(source))?;
                    }
                    ArithmeticExpansionSyntax::LegacyBracket => {
                        std::write!(rendered, "$[{}]", expression.slice(source))?;
                    }
                }
            }
        }
        WordPart::Parameter(parameter) => {
            rendered.push_str(&render_parameter_word(parameter, source, options));
        }
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            colon_variant,
        } => render_parameter_expansion(
            rendered,
            reference,
            operator.clone(),
            operand.as_ref(),
            *colon_variant,
            source,
            options,
        )?,
        WordPart::Length(reference) => {
            std::write!(
                rendered,
                "${{#{}",
                render_var_ref(reference, source, options)
            )?;
            rendered.push('}');
        }
        WordPart::ArrayAccess(reference) => {
            std::write!(
                rendered,
                "${{{}}}",
                render_var_ref(reference, source, options)
            )?;
        }
        WordPart::ArrayLength(reference) => {
            std::write!(
                rendered,
                "${{#{}",
                render_var_ref(reference, source, options)
            )?;
            rendered.push('}');
        }
        WordPart::ArrayIndices(reference) => {
            std::write!(
                rendered,
                "${{!{}",
                render_var_ref(reference, source, options)
            )?;
            rendered.push('}');
        }
        WordPart::Substring {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
        } => {
            std::write!(
                rendered,
                "${{{}",
                render_var_ref(reference, source, options)
            )?;
            rendered.push(':');
            rendered.push_str(&render_arithmetic_source_text(
                offset,
                offset_ast.as_ref(),
                source,
                options,
            ));
            if let Some(length) = length {
                rendered.push(':');
                rendered.push_str(&render_arithmetic_source_text(
                    length,
                    length_ast.as_ref(),
                    source,
                    options,
                ));
            }
            rendered.push('}');
        }
        WordPart::IndirectExpansion {
            name,
            operator,
            operand,
            colon_variant,
        } => {
            rendered.push_str("${!");
            rendered.push_str(name.as_ref());
            if let Some(operator) = operator {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(parameter_defaulting_operator(operator.clone()));
                if let Some(operand) = operand {
                    rendered.push_str(operand.slice(source));
                }
            }
            rendered.push('}');
        }
        WordPart::PrefixMatch { prefix, kind } => {
            std::write!(rendered, "${{!{}{}}}", prefix, kind.as_char())?;
        }
        WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => {
            rendered.push_str(span.slice(source));
        }
    }

    Ok(())
}

fn render_command_substitution(
    body: &shuck_ast::StmtSeq,
    source: &str,
    options: &ResolvedShellFormatOptions,
    multiline: bool,
) -> Option<String> {
    let context = ShellFormatContext::new(options.clone(), source, Comments::from_ast(source, &[]));
    let mut formatter = shuck_format::Formatter::new(context);
    format_stmt_sequence(body, &mut formatter).ok()?;
    let rendered = formatter.finish().print().ok()?.into_code();
    let trimmed = rendered.trim_end_matches('\n');
    if trimmed.is_empty() {
        return Some("$()".to_string());
    }

    if multiline {
        Some(format!(
            "$(\n{}\n)",
            indent_rendered_block(trimmed, options, 1)
        ))
    } else {
        Some(format!("$({trimmed})"))
    }
}

fn indent_rendered_block(
    rendered: &str,
    options: &ResolvedShellFormatOptions,
    levels: usize,
) -> String {
    let prefix = match options.indent_style() {
        IndentStyle::Tab => "\t".repeat(levels),
        IndentStyle::Space => " ".repeat(levels * usize::from(options.indent_width())),
    };

    rendered
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_double_quoted_literal(rendered: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '"' | '\\' | '$' | '`' => {
                rendered.push('\\');
                rendered.push(ch);
            }
            _ => rendered.push(ch),
        }
    }
}

fn render_arithmetic_expr(
    expr: &ArithmeticExprNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    render_arithmetic_expr_with_parent(expr, ArithmeticContext::TopLevel, source, options)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArithmeticContext {
    TopLevel,
    Unary,
    Postfix,
    Binary(ArithmeticBinaryOp),
    Assignment,
    ConditionalCondition,
    ConditionalBranch,
    Subscript,
}

fn render_arithmetic_expr_with_parent(
    expr: &ArithmeticExprNode,
    context: ArithmeticContext,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let rendered = match &expr.kind {
        ArithmeticExpr::Number(number) => number.slice(source).to_string(),
        ArithmeticExpr::Variable(name) => name.to_string(),
        ArithmeticExpr::Indexed { name, index } => format!(
            "{}[{}]",
            name,
            render_arithmetic_expr_with_parent(
                index,
                ArithmeticContext::Subscript,
                source,
                options
            )
        ),
        ArithmeticExpr::ShellWord(word) => render_arithmetic_shell_word(word, source, options),
        ArithmeticExpr::Parenthesized { expression } => format!(
            "({})",
            render_arithmetic_expr_with_parent(
                expression,
                ArithmeticContext::TopLevel,
                source,
                options
            )
        ),
        ArithmeticExpr::Unary { op, expr } => format!(
            "{}{}",
            arithmetic_unary_operator(*op),
            render_arithmetic_expr_with_parent(expr, ArithmeticContext::Unary, source, options)
        ),
        ArithmeticExpr::Postfix { expr, op } => format!(
            "{}{}",
            render_arithmetic_expr_with_parent(expr, ArithmeticContext::Postfix, source, options),
            arithmetic_postfix_operator(*op)
        ),
        ArithmeticExpr::Binary { left, op, right } => format!(
            "{} {} {}",
            render_arithmetic_expr_with_parent(
                left,
                ArithmeticContext::Binary(*op),
                source,
                options
            ),
            arithmetic_binary_operator(*op),
            render_arithmetic_expr_with_parent(
                right,
                ArithmeticContext::Binary(*op),
                source,
                options
            )
        ),
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => format!(
            "{} ? {} : {}",
            render_arithmetic_expr_with_parent(
                condition,
                ArithmeticContext::ConditionalCondition,
                source,
                options
            ),
            render_arithmetic_expr_with_parent(
                then_expr,
                ArithmeticContext::ConditionalBranch,
                source,
                options
            ),
            render_arithmetic_expr_with_parent(
                else_expr,
                ArithmeticContext::ConditionalBranch,
                source,
                options
            )
        ),
        ArithmeticExpr::Assignment { target, op, value } => format!(
            "{} {} {}",
            render_arithmetic_lvalue(target, source, options),
            arithmetic_assign_operator(*op),
            render_arithmetic_expr_with_parent(
                value,
                ArithmeticContext::Assignment,
                source,
                options
            )
        ),
    };

    if arithmetic_needs_parentheses(expr, context) {
        format!("({rendered})")
    } else {
        rendered
    }
}

fn render_arithmetic_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let [part] = word.parts.as_slice() else {
        return render_word_syntax(word, source, options);
    };

    match &part.kind {
        WordPart::Variable(name) => name.to_string(),
        WordPart::ArrayAccess(reference) if reference.subscript.is_none() => {
            reference.name.to_string()
        }
        WordPart::Parameter(parameter)
            if is_plain_arithmetic_identifier(parameter.raw_body.slice(source)) =>
        {
            parameter.raw_body.slice(source).to_string()
        }
        _ => render_word_syntax(word, source, options),
    }
}

fn is_plain_arithmetic_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn arithmetic_needs_parentheses(expr: &ArithmeticExprNode, context: ArithmeticContext) -> bool {
    let expr_prec = arithmetic_precedence(expr);
    match context {
        ArithmeticContext::TopLevel
        | ArithmeticContext::Subscript
        | ArithmeticContext::ConditionalCondition => false,
        ArithmeticContext::Unary => {
            expr_prec < arithmetic_precedence_value(ArithmeticBinaryOp::Power)
        }
        ArithmeticContext::Postfix => {
            expr_prec < arithmetic_precedence_value(ArithmeticBinaryOp::Power)
        }
        ArithmeticContext::Assignment => expr_prec <= 2,
        ArithmeticContext::ConditionalBranch => expr_prec <= 1,
        ArithmeticContext::Binary(parent_op) => {
            let parent_prec = arithmetic_precedence_value(parent_op);
            expr_prec < parent_prec
                || matches!(
                    expr.kind,
                    ArithmeticExpr::Assignment { .. } | ArithmeticExpr::Conditional { .. }
                ) && expr_prec == parent_prec
        }
    }
}

fn arithmetic_precedence(expr: &ArithmeticExprNode) -> u8 {
    match &expr.kind {
        ArithmeticExpr::Number(_)
        | ArithmeticExpr::Variable(_)
        | ArithmeticExpr::Indexed { .. }
        | ArithmeticExpr::ShellWord(_)
        | ArithmeticExpr::Parenthesized { .. } => 100,
        ArithmeticExpr::Postfix { .. } => 90,
        ArithmeticExpr::Unary { .. } => 80,
        ArithmeticExpr::Binary { op, .. } => arithmetic_precedence_value(*op),
        ArithmeticExpr::Conditional { .. } => 2,
        ArithmeticExpr::Assignment { .. } => 1,
    }
}

fn arithmetic_precedence_value(op: ArithmeticBinaryOp) -> u8 {
    match op {
        ArithmeticBinaryOp::Comma => 0,
        ArithmeticBinaryOp::LogicalOr => 10,
        ArithmeticBinaryOp::LogicalAnd => 20,
        ArithmeticBinaryOp::BitwiseOr => 30,
        ArithmeticBinaryOp::BitwiseXor => 40,
        ArithmeticBinaryOp::BitwiseAnd => 50,
        ArithmeticBinaryOp::Equal | ArithmeticBinaryOp::NotEqual => 60,
        ArithmeticBinaryOp::LessThan
        | ArithmeticBinaryOp::LessThanOrEqual
        | ArithmeticBinaryOp::GreaterThan
        | ArithmeticBinaryOp::GreaterThanOrEqual => 70,
        ArithmeticBinaryOp::ShiftLeft | ArithmeticBinaryOp::ShiftRight => 75,
        ArithmeticBinaryOp::Add | ArithmeticBinaryOp::Subtract => 80,
        ArithmeticBinaryOp::Multiply | ArithmeticBinaryOp::Divide | ArithmeticBinaryOp::Modulo => {
            85
        }
        ArithmeticBinaryOp::Power => 95,
    }
}

fn arithmetic_unary_operator(op: ArithmeticUnaryOp) -> &'static str {
    match op {
        ArithmeticUnaryOp::PreIncrement => "++",
        ArithmeticUnaryOp::PreDecrement => "--",
        ArithmeticUnaryOp::Plus => "+",
        ArithmeticUnaryOp::Minus => "-",
        ArithmeticUnaryOp::LogicalNot => "!",
        ArithmeticUnaryOp::BitwiseNot => "~",
    }
}

fn arithmetic_postfix_operator(op: ArithmeticPostfixOp) -> &'static str {
    match op {
        ArithmeticPostfixOp::Increment => "++",
        ArithmeticPostfixOp::Decrement => "--",
    }
}

fn arithmetic_binary_operator(op: ArithmeticBinaryOp) -> &'static str {
    match op {
        ArithmeticBinaryOp::Comma => ",",
        ArithmeticBinaryOp::Power => "**",
        ArithmeticBinaryOp::Multiply => "*",
        ArithmeticBinaryOp::Divide => "/",
        ArithmeticBinaryOp::Modulo => "%",
        ArithmeticBinaryOp::Add => "+",
        ArithmeticBinaryOp::Subtract => "-",
        ArithmeticBinaryOp::ShiftLeft => "<<",
        ArithmeticBinaryOp::ShiftRight => ">>",
        ArithmeticBinaryOp::LessThan => "<",
        ArithmeticBinaryOp::LessThanOrEqual => "<=",
        ArithmeticBinaryOp::GreaterThan => ">",
        ArithmeticBinaryOp::GreaterThanOrEqual => ">=",
        ArithmeticBinaryOp::Equal => "==",
        ArithmeticBinaryOp::NotEqual => "!=",
        ArithmeticBinaryOp::BitwiseAnd => "&",
        ArithmeticBinaryOp::BitwiseXor => "^",
        ArithmeticBinaryOp::BitwiseOr => "|",
        ArithmeticBinaryOp::LogicalAnd => "&&",
        ArithmeticBinaryOp::LogicalOr => "||",
    }
}

fn arithmetic_assign_operator(op: ArithmeticAssignOp) -> &'static str {
    match op {
        ArithmeticAssignOp::Assign => "=",
        ArithmeticAssignOp::AddAssign => "+=",
        ArithmeticAssignOp::SubAssign => "-=",
        ArithmeticAssignOp::MulAssign => "*=",
        ArithmeticAssignOp::DivAssign => "/=",
        ArithmeticAssignOp::ModAssign => "%=",
        ArithmeticAssignOp::ShiftLeftAssign => "<<=",
        ArithmeticAssignOp::ShiftRightAssign => ">>=",
        ArithmeticAssignOp::AndAssign => "&=",
        ArithmeticAssignOp::XorAssign => "^=",
        ArithmeticAssignOp::OrAssign => "|=",
    }
}

fn render_arithmetic_lvalue(
    target: &ArithmeticLvalue,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    match target {
        ArithmeticLvalue::Variable(name) => name.to_string(),
        ArithmeticLvalue::Indexed { name, index } => format!(
            "{}[{}]",
            name,
            render_arithmetic_expr_with_parent(
                index,
                ArithmeticContext::Subscript,
                source,
                options
            )
        ),
    }
}

fn render_arithmetic_source_text(
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    ast.map(|ast| render_arithmetic_expr(ast, source, options))
        .unwrap_or_else(|| text.slice(source).to_string())
}

fn render_var_ref(
    reference: &VarRef,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let mut rendered = reference.name.to_string();
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        if let Some(selector) = subscript.selector() {
            rendered.push(match selector {
                SubscriptSelector::At => '@',
                SubscriptSelector::Star => '*',
            });
        } else if let Some(ast) = subscript.arithmetic_ast.as_ref() {
            rendered.push_str(&render_arithmetic_expr(ast, source, options));
        } else {
            rendered.push_str(subscript.syntax_text(source));
        }
        rendered.push(']');
    }
    rendered
}

fn render_parameter_word(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let Some(syntax) = parameter.bourne() else {
        return format!("${{{}}}", parameter.raw_body.slice(source));
    };

    match syntax {
        BourneParameterExpansion::Access { reference } => {
            format!("${{{}}}", render_var_ref(reference, source, options))
        }
        BourneParameterExpansion::Length { reference } => {
            format!("${{#{}}}", render_var_ref(reference, source, options))
        }
        BourneParameterExpansion::Indices { reference } => {
            format!("${{!{}}}", render_var_ref(reference, source, options))
        }
        BourneParameterExpansion::Indirect {
            name,
            operator,
            operand,
            colon_variant,
        } => {
            let mut rendered = format!("${{!{name}");
            if let Some(operator) = operator {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(parameter_defaulting_operator(operator.clone()));
                if let Some(operand) = operand {
                    rendered.push_str(operand.slice(source));
                }
            }
            rendered.push('}');
            rendered
        }
        BourneParameterExpansion::PrefixMatch { prefix, kind } => {
            format!("${{!{}{}}}", prefix, kind.as_char())
        }
        BourneParameterExpansion::Slice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
        } => {
            let mut rendered = format!("${{{}", render_var_ref(reference, source, options));
            rendered.push(':');
            rendered.push_str(&render_arithmetic_source_text(
                offset,
                offset_ast.as_ref(),
                source,
                options,
            ));
            if let Some(length) = length {
                rendered.push(':');
                rendered.push_str(&render_arithmetic_source_text(
                    length,
                    length_ast.as_ref(),
                    source,
                    options,
                ));
            }
            rendered.push('}');
            rendered
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            colon_variant,
        } => {
            let mut rendered = String::new();
            render_parameter_expansion(
                &mut rendered,
                reference,
                operator.clone(),
                operand.as_ref(),
                *colon_variant,
                source,
                options,
            )
            .expect("writing into a String should not fail");
            rendered
        }
        BourneParameterExpansion::Transformation {
            reference,
            operator,
        } => {
            format!(
                "${{{}@{operator}}}",
                render_var_ref(reference, source, options)
            )
        }
    }
}

fn render_parameter_expansion(
    rendered: &mut String,
    reference: &VarRef,
    operator: ParameterOp,
    operand: Option<&shuck_ast::SourceText>,
    colon_variant: bool,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Result<(), std::fmt::Error> {
    rendered.push_str("${");
    rendered.push_str(&render_var_ref(reference, source, options));
    match operator {
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error => {
            if colon_variant {
                rendered.push(':');
            }
            rendered.push_str(parameter_defaulting_operator(operator));
            if let Some(operand) = operand {
                rendered.push_str(operand.slice(source));
            }
        }
        ParameterOp::RemovePrefixShort { pattern } => {
            rendered.push('#');
            rendered.push_str(&render_pattern_syntax(&pattern, source, options));
        }
        ParameterOp::RemovePrefixLong { pattern } => {
            rendered.push_str("##");
            rendered.push_str(&render_pattern_syntax(&pattern, source, options));
        }
        ParameterOp::RemoveSuffixShort { pattern } => {
            rendered.push('%');
            rendered.push_str(&render_pattern_syntax(&pattern, source, options));
        }
        ParameterOp::RemoveSuffixLong { pattern } => {
            rendered.push_str("%%");
            rendered.push_str(&render_pattern_syntax(&pattern, source, options));
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        } => {
            rendered.push('/');
            rendered.push_str(&render_pattern_syntax(&pattern, source, options));
            rendered.push('/');
            rendered.push_str(replacement.slice(source));
        }
        ParameterOp::ReplaceAll {
            pattern,
            replacement,
        } => {
            rendered.push_str("//");
            rendered.push_str(&render_pattern_syntax(&pattern, source, options));
            rendered.push('/');
            rendered.push_str(replacement.slice(source));
        }
        ParameterOp::UpperFirst => rendered.push('^'),
        ParameterOp::UpperAll => rendered.push_str("^^"),
        ParameterOp::LowerFirst => rendered.push(','),
        ParameterOp::LowerAll => rendered.push_str(",,"),
    }
    rendered.push('}');
    Ok(())
}

fn parameter_defaulting_operator(operator: ParameterOp) -> &'static str {
    match operator {
        ParameterOp::UseDefault => "-",
        ParameterOp::AssignDefault => "=",
        ParameterOp::UseReplacement => "+",
        ParameterOp::Error => "?",
        _ => "",
    }
}

pub(crate) fn render_pattern_syntax(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let rendered = pattern.render_syntax(source);

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_pattern_source_slice(pattern, source)
        && should_preserve_raw_syntax(slice, &rendered)
    {
        return slice.to_string();
    }

    rendered
}

fn raw_word_source_slice<'a>(word: &Word, source: &'a str) -> Option<&'a str> {
    raw_source_slice(word.span, source)
}

fn raw_pattern_source_slice<'a>(pattern: &Pattern, source: &'a str) -> Option<&'a str> {
    raw_source_slice(pattern.span, source)
}

fn raw_source_slice(span: shuck_ast::Span, source: &str) -> Option<&str> {
    if span.start.offset >= span.end.offset || span.end.offset > source.len() {
        return None;
    }

    let slice = span.slice(source);
    if slice.contains('\n') {
        Some(slice)
    } else {
        Some(trim_unescaped_trailing_whitespace(slice))
    }
}

fn should_preserve_raw_syntax(raw: &str, rendered: &str) -> bool {
    raw != rendered
        && (raw.starts_with('\\')
            || raw.starts_with('&')
            || raw.starts_with("$'")
            || raw.contains("\\\"")
            || raw.contains("\\`")
            || raw.contains("\\\\")
            || raw.contains("[^ ]"))
}

fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text.as_bytes()[..whitespace_start]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}
