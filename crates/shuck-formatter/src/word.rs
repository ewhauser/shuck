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
use crate::command::stmt_seq_has_heredoc;
use crate::comments::SourceMap;
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::prelude::ShellFormatter;
use crate::streaming::format_stmt_sequence_streaming_to_buf;

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
    let mut rendered = String::new();
    render_word_syntax_to_buf(word, source, options, &mut rendered);
    rendered
}

pub(crate) fn render_word_syntax_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    rendered: &mut String,
) {
    render_word_syntax_internal(word, source, options, None, None, rendered);
}

pub(crate) fn render_word_syntax_with_facts_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
    rendered: &mut String,
) {
    render_word_syntax_internal(
        word,
        source,
        options,
        Some(source_map),
        Some(facts),
        rendered,
    );
}

fn render_word_syntax_internal(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    rendered: &mut String,
) {
    if word_needs_special_rendering(word) {
        render_word_parts(
            word.parts.as_slice(),
            source,
            options,
            source_map,
            facts,
            rendered,
        )
        .expect("writing into a String should not fail");
        return;
    }

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_word_source_slice(word, source)
        && could_need_preserve_raw_syntax(slice)
    {
        let start = rendered.len();
        word.render_syntax_to_buf(source, rendered);
        if should_preserve_raw_syntax(slice, &rendered[start..]) {
            rendered.truncate(start);
            rendered.push_str(slice);
        }
        return;
    }

    word.render_syntax_to_buf(source, rendered);
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
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    rendered: &mut String,
) -> Result<(), std::fmt::Error> {
    for part in parts {
        render_word_part(
            rendered, &part.kind, part.span, source, options, source_map, facts,
        )?;
    }
    Ok(())
}

fn render_word_part(
    rendered: &mut String,
    part: &WordPart,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
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
                    other => render_word_part(
                        rendered, other, part.span, source, options, source_map, facts,
                    )?,
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
                } else if render_command_substitution(
                    rendered,
                    body,
                    span.end.offset,
                    source,
                    options,
                    raw.contains('\n'),
                    source_map,
                    facts,
                )
                .is_some()
                {
                } else {
                    rendered.push_str(raw);
                }
            } else if render_command_substitution(
                rendered,
                body,
                span.end.offset,
                source,
                options,
                true,
                source_map,
                facts,
            )
            .is_some()
            {
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
                        push_arithmetic_expr(
                            rendered,
                            expression_ast,
                            ArithmeticContext::TopLevel,
                            source,
                            options,
                        );
                        rendered.push_str("))");
                    }
                    ArithmeticExpansionSyntax::LegacyBracket => {
                        rendered.push_str("$[");
                        push_arithmetic_expr(
                            rendered,
                            expression_ast,
                            ArithmeticContext::TopLevel,
                            source,
                            options,
                        );
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
            push_parameter_word(rendered, parameter, source, options)?;
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
            rendered.push_str("${#");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        WordPart::ArrayAccess(reference) => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        WordPart::ArrayLength(reference) => {
            rendered.push_str("${#");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        WordPart::ArrayIndices(reference) => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
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
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push(':');
            push_arithmetic_source_text(rendered, offset, offset_ast.as_ref(), source, options);
            if let Some(length) = length {
                rendered.push(':');
                push_arithmetic_source_text(rendered, length, length_ast.as_ref(), source, options);
            }
            rendered.push('}');
        }
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            colon_variant,
        } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
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

#[allow(clippy::too_many_arguments)]
fn render_command_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    upper_bound: usize,
    source: &str,
    options: &ResolvedShellFormatOptions,
    multiline: bool,
    _source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> Option<()> {
    if stmt_seq_has_heredoc(body) {
        return None;
    }

    let mut nested = String::new();
    let owned_facts;
    let facts = match facts {
        Some(facts) => facts,
        None => {
            let file = shuck_ast::File {
                body: body.clone(),
                span: body.span,
            };
            owned_facts = FormatterFacts::build(source, &file, options);
            &owned_facts
        }
    };
    format_stmt_sequence_streaming_to_buf(
        source,
        body,
        options,
        facts,
        Some(upper_bound),
        &mut nested,
    )
    .ok()?;

    let trimmed = trim_trailing_line_endings(&nested);
    if trimmed.is_empty() {
        rendered.push_str("$()");
        return Some(());
    }

    if multiline {
        rendered.push_str("$(\n");
        push_indented_rendered_block(rendered, trimmed, options, 1);
        rendered.push_str("\n)");
    } else {
        rendered.push_str("$(");
        rendered.push_str(trimmed);
        rendered.push(')');
    }

    Some(())
}

fn trim_trailing_line_endings(rendered: &str) -> &str {
    rendered.trim_end_matches(&['\r', '\n'][..])
}

fn push_indented_rendered_block(
    target: &mut String,
    rendered: &str,
    options: &ResolvedShellFormatOptions,
    levels: usize,
) {
    let prefix = match options.indent_style() {
        IndentStyle::Tab => "\t".repeat(levels),
        IndentStyle::Space => " ".repeat(levels * usize::from(options.indent_width())),
    };

    for (index, line) in rendered.lines().enumerate() {
        if index > 0 {
            target.push('\n');
        }
        if !line.is_empty() {
            target.push_str(&prefix);
            target.push_str(line);
        }
    }
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

fn render_arithmetic_expr_to_buf(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    push_arithmetic_expr(rendered, expr, ArithmeticContext::TopLevel, source, options);
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

fn push_arithmetic_expr(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    context: ArithmeticContext,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let needs_parentheses = arithmetic_needs_parentheses(expr, context);
    if needs_parentheses {
        rendered.push('(');
    }

    match &expr.kind {
        ArithmeticExpr::Number(number) => rendered.push_str(number.slice(source)),
        ArithmeticExpr::Variable(name) => rendered.push_str(name),
        ArithmeticExpr::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(
                rendered,
                index,
                ArithmeticContext::Subscript,
                source,
                options,
            );
            rendered.push(']');
        }
        ArithmeticExpr::ShellWord(word) => {
            rendered.push_str(&render_arithmetic_shell_word(word, source, options));
        }
        ArithmeticExpr::Parenthesized { expression } => {
            rendered.push('(');
            push_arithmetic_expr(
                rendered,
                expression,
                ArithmeticContext::TopLevel,
                source,
                options,
            );
            rendered.push(')');
        }
        ArithmeticExpr::Unary { op, expr } => {
            rendered.push_str(arithmetic_unary_operator(*op));
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Unary, source, options);
        }
        ArithmeticExpr::Postfix { expr, op } => {
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Postfix, source, options);
            rendered.push_str(arithmetic_postfix_operator(*op));
        }
        ArithmeticExpr::Binary { left, op, right } => {
            push_arithmetic_expr(
                rendered,
                left,
                ArithmeticContext::Binary(*op),
                source,
                options,
            );
            rendered.push(' ');
            rendered.push_str(arithmetic_binary_operator(*op));
            rendered.push(' ');
            push_arithmetic_expr(
                rendered,
                right,
                ArithmeticContext::Binary(*op),
                source,
                options,
            );
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            push_arithmetic_expr(
                rendered,
                condition,
                ArithmeticContext::ConditionalCondition,
                source,
                options,
            );
            rendered.push_str(" ? ");
            push_arithmetic_expr(
                rendered,
                then_expr,
                ArithmeticContext::ConditionalBranch,
                source,
                options,
            );
            rendered.push_str(" : ");
            push_arithmetic_expr(
                rendered,
                else_expr,
                ArithmeticContext::ConditionalBranch,
                source,
                options,
            );
        }
        ArithmeticExpr::Assignment { target, op, value } => {
            push_arithmetic_lvalue(rendered, target, source, options);
            rendered.push(' ');
            rendered.push_str(arithmetic_assign_operator(*op));
            rendered.push(' ');
            push_arithmetic_expr(
                rendered,
                value,
                ArithmeticContext::Assignment,
                source,
                options,
            );
        }
    }

    if needs_parentheses {
        rendered.push(')');
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

fn push_arithmetic_lvalue(
    rendered: &mut String,
    target: &ArithmeticLvalue,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    match target {
        ArithmeticLvalue::Variable(name) => rendered.push_str(name),
        ArithmeticLvalue::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(
                rendered,
                index,
                ArithmeticContext::Subscript,
                source,
                options,
            );
            rendered.push(']');
        }
    }
}

fn push_arithmetic_source_text(
    rendered: &mut String,
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    if let Some(ast) = ast {
        render_arithmetic_expr_to_buf(rendered, ast, source, options);
    } else {
        rendered.push_str(text.slice(source));
    }
}

fn push_var_ref(
    rendered: &mut String,
    reference: &VarRef,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    rendered.push_str(reference.name.as_ref());
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        if let Some(selector) = subscript.selector() {
            rendered.push(match selector {
                SubscriptSelector::At => '@',
                SubscriptSelector::Star => '*',
            });
        } else if let Some(ast) = subscript.arithmetic_ast.as_ref() {
            render_arithmetic_expr_to_buf(rendered, ast, source, options);
        } else {
            rendered.push_str(subscript.syntax_text(source));
        }
        rendered.push(']');
    }
}

fn push_parameter_word(
    rendered: &mut String,
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Result<(), std::fmt::Error> {
    let Some(syntax) = parameter.bourne() else {
        let raw = parameter.raw_body.slice(source);
        rendered.push_str("${");
        rendered.push_str(raw);
        rendered.push('}');
        return Ok(());
    };

    match syntax {
        BourneParameterExpansion::Access { reference } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        BourneParameterExpansion::Length { reference } => {
            rendered.push_str("${#");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        BourneParameterExpansion::Indices { reference } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        BourneParameterExpansion::Indirect {
            reference,
            operator,
            operand,
            colon_variant,
        } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
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
        BourneParameterExpansion::PrefixMatch { prefix, kind } => {
            rendered.push_str("${!");
            rendered.push_str(prefix);
            rendered.push(kind.as_char());
            rendered.push('}');
        }
        BourneParameterExpansion::Slice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
        } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push(':');
            push_arithmetic_source_text(rendered, offset, offset_ast.as_ref(), source, options);
            if let Some(length) = length {
                rendered.push(':');
                push_arithmetic_source_text(rendered, length, length_ast.as_ref(), source, options);
            }
            rendered.push('}');
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            colon_variant,
        } => {
            render_parameter_expansion(
                rendered,
                reference,
                operator.clone(),
                operand.as_ref(),
                *colon_variant,
                source,
                options,
            )?;
        }
        BourneParameterExpansion::Transformation {
            reference,
            operator,
        } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push('@');
            std::write!(rendered, "{operator}")?;
            rendered.push('}');
        }
    }

    Ok(())
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
    push_var_ref(rendered, reference, source, options);
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
            render_pattern_syntax_to_buf(&pattern, source, options, rendered);
        }
        ParameterOp::RemovePrefixLong { pattern } => {
            rendered.push_str("##");
            render_pattern_syntax_to_buf(&pattern, source, options, rendered);
        }
        ParameterOp::RemoveSuffixShort { pattern } => {
            rendered.push('%');
            render_pattern_syntax_to_buf(&pattern, source, options, rendered);
        }
        ParameterOp::RemoveSuffixLong { pattern } => {
            rendered.push_str("%%");
            render_pattern_syntax_to_buf(&pattern, source, options, rendered);
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        } => {
            rendered.push('/');
            render_pattern_syntax_to_buf(&pattern, source, options, rendered);
            rendered.push('/');
            rendered.push_str(replacement.slice(source));
        }
        ParameterOp::ReplaceAll {
            pattern,
            replacement,
        } => {
            rendered.push_str("//");
            render_pattern_syntax_to_buf(&pattern, source, options, rendered);
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
    let mut rendered = String::new();
    render_pattern_syntax_to_buf(pattern, source, options, &mut rendered);
    rendered
}

pub(crate) fn render_pattern_syntax_to_buf(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
    rendered: &mut String,
) {
    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_pattern_source_slice(pattern, source)
        && could_need_preserve_raw_syntax(slice)
    {
        let start = rendered.len();
        pattern.render_syntax_to_buf(source, rendered);
        if should_preserve_raw_syntax(slice, &rendered[start..]) {
            rendered.truncate(start);
            rendered.push_str(slice);
        }
        return;
    }

    pattern.render_syntax_to_buf(source, rendered);
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
    raw != rendered && could_need_preserve_raw_syntax(raw)
}

fn could_need_preserve_raw_syntax(raw: &str) -> bool {
    raw.starts_with('\\')
        || raw.starts_with('&')
        || raw.starts_with("$'")
        || raw.contains("\\\"")
        || raw.contains("\\`")
        || raw.contains("\\\\")
        || raw.contains("[^ ]")
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
