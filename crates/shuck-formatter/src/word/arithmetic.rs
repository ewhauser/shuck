use super::core::*;
use super::*;

pub(crate) fn render_arithmetic_expr_to_buf(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    push_arithmetic_expr(
        rendered,
        expr,
        ArithmeticContext::TopLevel,
        false,
        WordRenderEnv::new(source, options, None, None),
    );
}

pub(super) fn render_arithmetic_subscript_expr_to_buf(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
    compact: bool,
) {
    push_arithmetic_expr(
        rendered,
        expr,
        ArithmeticContext::Subscript,
        compact,
        WordRenderEnv::new(source, options, None, None),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ArithmeticContext {
    TopLevel,
    Unary,
    Postfix,
    Binary(ArithmeticBinaryOp),
    Assignment,
    ConditionalCondition,
    ConditionalBranch,
    Subscript,
}

pub(super) fn push_arithmetic_expr(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    context: ArithmeticContext,
    compact: bool,
    env: WordRenderEnv<'_, '_>,
) {
    let needs_parentheses = arithmetic_needs_parentheses(expr, context);
    if needs_parentheses {
        rendered.push('(');
    }

    match &expr.kind {
        ArithmeticExpr::Number(number) => rendered.push_str(number.slice(env.source)),
        ArithmeticExpr::Variable(name) => rendered.push_str(name),
        ArithmeticExpr::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(rendered, index, ArithmeticContext::Subscript, true, env);
            rendered.push(']');
        }
        ArithmeticExpr::ShellWord(word) => {
            let word = render_arithmetic_shell_word(
                word,
                env.source,
                env.options,
                env.source_map,
                env.facts,
            );
            if compact {
                rendered.push_str(&compact_dynamic_arithmetic_subscript(
                    word.trim_matches([' ', '\t', '\r']),
                ));
            } else {
                rendered.push_str(&word);
            }
        }
        ArithmeticExpr::Parenthesized { expression } => {
            rendered.push('(');
            push_arithmetic_expr(
                rendered,
                expression,
                ArithmeticContext::TopLevel,
                compact,
                env,
            );
            rendered.push(')');
        }
        ArithmeticExpr::Unary { op, expr } => {
            rendered.push_str(arithmetic_unary_operator(*op));
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Unary, compact, env);
        }
        ArithmeticExpr::Postfix { expr, op } => {
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Postfix, compact, env);
            rendered.push_str(arithmetic_postfix_operator(*op));
        }
        ArithmeticExpr::Binary { left, op, right } => {
            push_arithmetic_expr(rendered, left, ArithmeticContext::Binary(*op), compact, env);
            if !compact {
                rendered.push(' ');
            }
            rendered.push_str(arithmetic_binary_operator(*op));
            if !compact {
                rendered.push(' ');
            }
            push_arithmetic_expr(
                rendered,
                right,
                ArithmeticContext::Binary(*op),
                compact,
                env,
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
                compact,
                env,
            );
            rendered.push_str(if compact { "?" } else { " ? " });
            push_arithmetic_expr(
                rendered,
                then_expr,
                ArithmeticContext::ConditionalBranch,
                compact,
                env,
            );
            rendered.push_str(if compact { ":" } else { " : " });
            push_arithmetic_expr(
                rendered,
                else_expr,
                ArithmeticContext::ConditionalBranch,
                compact,
                env,
            );
        }
        ArithmeticExpr::Assignment { target, op, value } => {
            push_arithmetic_lvalue(rendered, target, env);
            if !compact {
                rendered.push(' ');
            }
            rendered.push_str(arithmetic_assign_operator(*op));
            if !compact {
                rendered.push(' ');
            }
            push_arithmetic_expr(rendered, value, ArithmeticContext::Assignment, compact, env);
        }
    }

    if needs_parentheses {
        rendered.push(')');
    }
}

pub(super) fn push_arithmetic_expansion_body(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    env: WordRenderEnv<'_, '_>,
) {
    let mut body = String::new();
    push_arithmetic_expr(&mut body, expr, ArithmeticContext::TopLevel, false, env);
    if body.contains("$(")
        || body.contains('`')
        || arithmetic_expr_contains_command_substitution(expr)
    {
        rendered.push_str(body.trim_matches([' ', '\t', '\r']));
    } else {
        rendered.push_str(&body);
    }
}

pub(super) fn arithmetic_expr_contains_command_substitution(expr: &ArithmeticExprNode) -> bool {
    match &expr.kind {
        ArithmeticExpr::ShellWord(word) => word.parts.iter().any(|part| {
            matches!(
                part.kind,
                WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. }
            )
        }),
        ArithmeticExpr::Indexed { index, .. } => {
            arithmetic_expr_contains_command_substitution(index)
        }
        ArithmeticExpr::Parenthesized { expression } => {
            arithmetic_expr_contains_command_substitution(expression)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            arithmetic_expr_contains_command_substitution(expr)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            arithmetic_expr_contains_command_substitution(left)
                || arithmetic_expr_contains_command_substitution(right)
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            arithmetic_expr_contains_command_substitution(condition)
                || arithmetic_expr_contains_command_substitution(then_expr)
                || arithmetic_expr_contains_command_substitution(else_expr)
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            arithmetic_lvalue_contains_command_substitution(target)
                || arithmetic_expr_contains_command_substitution(value)
        }
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => false,
    }
}

pub(super) fn arithmetic_lvalue_contains_command_substitution(target: &ArithmeticLvalue) -> bool {
    match target {
        ArithmeticLvalue::Variable(_) => false,
        ArithmeticLvalue::Indexed { index, .. } => {
            arithmetic_expr_contains_command_substitution(index)
        }
    }
}

pub(super) fn render_arithmetic_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> String {
    let render_with_context = || {
        let mut rendered = String::new();
        render_word_syntax_internal(
            word,
            source,
            options,
            source_map,
            facts,
            true,
            &mut rendered,
        );
        rendered
    };

    if options.simplify() || options.minify() {
        let [part] = word.parts.as_slice() else {
            return render_with_context();
        };

        return match &part.kind {
            WordPart::Variable(name) => name.to_string(),
            WordPart::ArrayAccess(reference) if reference.subscript.is_none() => {
                reference.name.to_string()
            }
            WordPart::Parameter(parameter)
                if is_plain_arithmetic_identifier(parameter.raw_body.slice(source)) =>
            {
                parameter.raw_body.slice(source).to_string()
            }
            _ => render_with_context(),
        };
    }

    render_with_context()
}

pub(super) fn is_plain_arithmetic_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(super) fn arithmetic_needs_parentheses(
    expr: &ArithmeticExprNode,
    context: ArithmeticContext,
) -> bool {
    let expr_prec = arithmetic_precedence(expr);
    match context {
        ArithmeticContext::TopLevel
        | ArithmeticContext::Subscript
        | ArithmeticContext::ConditionalCondition => false,
        ArithmeticContext::Unary | ArithmeticContext::Postfix => {
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

pub(super) fn arithmetic_precedence(expr: &ArithmeticExprNode) -> u8 {
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

pub(super) fn arithmetic_precedence_value(op: ArithmeticBinaryOp) -> u8 {
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

pub(super) fn arithmetic_unary_operator(op: ArithmeticUnaryOp) -> &'static str {
    match op {
        ArithmeticUnaryOp::PreIncrement => "++",
        ArithmeticUnaryOp::PreDecrement => "--",
        ArithmeticUnaryOp::Plus => "+",
        ArithmeticUnaryOp::Minus => "-",
        ArithmeticUnaryOp::LogicalNot => "!",
        ArithmeticUnaryOp::BitwiseNot => "~",
    }
}

pub(super) fn arithmetic_postfix_operator(op: ArithmeticPostfixOp) -> &'static str {
    match op {
        ArithmeticPostfixOp::Increment => "++",
        ArithmeticPostfixOp::Decrement => "--",
    }
}

pub(super) fn arithmetic_binary_operator(op: ArithmeticBinaryOp) -> &'static str {
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

pub(super) fn arithmetic_assign_operator(op: ArithmeticAssignOp) -> &'static str {
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

pub(super) fn push_arithmetic_lvalue(
    rendered: &mut String,
    target: &ArithmeticLvalue,
    env: WordRenderEnv<'_, '_>,
) {
    match target {
        ArithmeticLvalue::Variable(name) => rendered.push_str(name),
        ArithmeticLvalue::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(rendered, index, ArithmeticContext::Subscript, true, env);
            rendered.push(']');
        }
    }
}

pub(super) fn push_arithmetic_source_text(
    rendered: &mut String,
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    if let Some(ast) = ast {
        match &ast.kind {
            ArithmeticExpr::ShellWord(word) if !options.simplify() && !options.minify() => {
                rendered.push_str(&render_arithmetic_slice_shell_word(word, source, options));
            }
            _ => render_arithmetic_subscript_expr_to_buf(rendered, ast, source, options, true),
        }
    } else {
        rendered.push_str(text.slice(source));
    }
}

pub(super) fn push_parameter_slice_offset(
    rendered: &mut String,
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let mut offset = String::new();
    push_arithmetic_source_text(&mut offset, text, ast, source, options);
    if offset.starts_with('-') {
        rendered.push(' ');
    }
    rendered.push_str(&offset);
}

pub(super) fn render_arithmetic_slice_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let [part] = word.parts.as_slice() else {
        return render_word_syntax(word, source, options);
    };

    match &part.kind {
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => {
            let mut body = String::new();
            if let Some(ast) = expression_ast.as_deref() {
                render_arithmetic_expr_to_buf(&mut body, ast, source, options);
            } else {
                body.push_str(expression.slice(source).trim());
            }
            let (open, close) = match syntax {
                ArithmeticExpansionSyntax::DollarParenParen => ("$((", "))"),
                ArithmeticExpansionSyntax::LegacyBracket => ("$[", "]"),
            };
            format!("{open}{body}{close}")
        }
        _ => render_word_syntax(word, source, options),
    }
}

pub(super) fn push_arithmetic_expansion(
    rendered: &mut String,
    expression: &shuck_ast::SourceText,
    expression_ast: Option<&ArithmeticExprNode>,
    syntax: ArithmeticExpansionSyntax,
    env: WordRenderEnv<'_, '_>,
) {
    let expression_source = expression.slice(env.source);
    if matches!(syntax, ArithmeticExpansionSyntax::LegacyBracket) {
        push_trimmed_arithmetic_expansion_source(rendered, expression_source, syntax);
    } else if let Some(formatted) = format_multiline_arithmetic_expansion_source(
        expression_source,
        syntax,
        expression_ast,
        env.source,
        env.options,
    ) {
        rendered.push_str(&formatted);
    } else if arithmetic_expression_prefers_raw_source(expression_source)
        || !expression.is_source_backed()
    {
        push_trimmed_arithmetic_expansion_source(rendered, expression_source, syntax);
    } else if let Some(expression_ast) = expression_ast {
        match syntax {
            ArithmeticExpansionSyntax::DollarParenParen => {
                rendered.push_str("$((");
                push_arithmetic_expansion_body(rendered, expression_ast, env);
                rendered.push_str("))");
            }
            ArithmeticExpansionSyntax::LegacyBracket => unreachable!("handled above"),
        }
    } else {
        push_trimmed_arithmetic_expansion_source(rendered, expression_source, syntax);
    }
}

pub(super) fn push_trimmed_arithmetic_expansion_source(
    rendered: &mut String,
    expression_source: &str,
    syntax: ArithmeticExpansionSyntax,
) {
    match syntax {
        ArithmeticExpansionSyntax::DollarParenParen => {
            rendered.push_str("$((");
            rendered.push_str(expression_source.trim());
            rendered.push_str("))");
        }
        ArithmeticExpansionSyntax::LegacyBracket => {
            rendered.push_str("$[");
            rendered.push_str(expression_source.trim());
            rendered.push(']');
        }
    }
}

pub(super) fn format_multiline_arithmetic_expansion_source(
    expression_source: &str,
    syntax: ArithmeticExpansionSyntax,
    expression_ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if !matches!(syntax, ArithmeticExpansionSyntax::DollarParenParen)
        || !expression_source.contains('\n')
        || expression_source.contains('`')
    {
        return None;
    }

    let operators = multiline_arithmetic_source_trailing_operators(expression_source)?;
    let mut rendered_body = String::new();
    if let Some(expression_ast) = expression_ast {
        render_arithmetic_expr_to_buf(&mut rendered_body, expression_ast, source, options);
    } else {
        rendered_body.push_str(expression_source.trim());
    }
    let lines = split_rendered_arithmetic_body_at_source_operators(&rendered_body, &operators)?;
    let mut continuation_indent = String::new();
    options.push_indent_units(&mut continuation_indent, 1);
    let lines = lines
        .into_iter()
        .map(|line| format!("{continuation_indent}{line}"))
        .collect::<Vec<_>>();
    Some(format!("$((\\\n{}))", lines.join(" \\\n")))
}

pub(super) fn multiline_arithmetic_source_trailing_operators(
    expression_source: &str,
) -> Option<Vec<&str>> {
    let lines = expression_source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return None;
    }

    let mut operators = Vec::with_capacity(lines.len().saturating_sub(1));
    for line in &lines[..lines.len() - 1] {
        operators.push(arithmetic_source_line_trailing_operator(line)?);
    }
    Some(operators)
}

pub(super) fn arithmetic_source_line_trailing_operator(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    [
        "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "**", "+", "-", "*", "/", "%", "<", ">",
        "&", "^", "|",
    ]
    .into_iter()
    .find(|operator| trimmed.ends_with(operator))
}

pub(super) fn split_rendered_arithmetic_body_at_source_operators(
    rendered_body: &str,
    operators: &[&str],
) -> Option<Vec<String>> {
    let mut lines = Vec::with_capacity(operators.len() + 1);
    let mut rest = rendered_body.trim();
    for operator in operators {
        let needle = format!(" {operator} ");
        let index = rest.find(&needle)?;
        let end = index + 1 + operator.len();
        lines.push(rest[..end].trim().to_string());
        rest = rest[index + needle.len()..].trim_start();
    }
    if rest.is_empty() {
        return None;
    }
    lines.push(rest.trim().to_string());
    Some(lines)
}

pub(super) fn arithmetic_expression_prefers_raw_source(expression_source: &str) -> bool {
    expression_source.contains('`')
}

pub(super) fn push_var_ref(
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
            let compact =
                !arithmetic_subscript_prefers_spaced_expression(subscript.syntax_text(source));
            render_arithmetic_subscript_expr_to_buf(rendered, ast, source, options, compact);
        } else {
            rendered.push_str(&compact_dynamic_arithmetic_subscript(
                subscript.syntax_text(source),
            ));
        }
        rendered.push(']');
    }
}

pub(super) fn arithmetic_subscript_prefers_spaced_expression(text: &str) -> bool {
    let text = text.trim_start_matches([' ', '\t', '\r']);
    text.starts_with("$((") || text.starts_with('(')
}

pub(super) fn compact_dynamic_arithmetic_subscript(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut dollar_paren_depth = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek().is_some_and(|next| *next == '(') {
            rendered.push(ch);
            rendered.push(chars.next().expect("peeked '('"));
            dollar_paren_depth = dollar_paren_depth.saturating_add(1);
            continue;
        }
        if dollar_paren_depth > 0 {
            if ch == '(' {
                dollar_paren_depth = dollar_paren_depth.saturating_add(1);
            } else if ch == ')' {
                dollar_paren_depth = dollar_paren_depth.saturating_sub(1);
            }
            rendered.push(ch);
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\r')
            && next_is_additive_operator_before_operand(chars.clone())
            && rendered
                .chars()
                .last()
                .is_some_and(|previous| !matches!(previous, ' ' | '\t' | '\r'))
        {
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\r')
            && chars.clone().next().is_some_and(|next| next == '%')
            && rendered
                .chars()
                .last()
                .is_some_and(|previous| !matches!(previous, ' ' | '\t' | '\r'))
        {
            continue;
        }
        if matches!(ch, '+' | '-')
            && chars
                .clone()
                .find(|next| !matches!(next, ' ' | '\t' | '\r'))
                .is_some_and(is_arithmetic_subscript_operand_start)
        {
            rendered.push(ch);
            while chars
                .peek()
                .is_some_and(|next| matches!(next, ' ' | '\t' | '\r'))
            {
                chars.next();
            }
            continue;
        }
        if ch == '%' {
            rendered.push(ch);
            while chars
                .peek()
                .is_some_and(|next| matches!(next, ' ' | '\t' | '\r'))
            {
                chars.next();
            }
            continue;
        }
        rendered.push(ch);
    }
    rendered
}

pub(super) fn next_is_additive_operator_before_operand(
    mut chars: std::iter::Peekable<std::str::Chars<'_>>,
) -> bool {
    let Some(operator) = chars.next() else {
        return false;
    };
    if !matches!(operator, '+' | '-') {
        return false;
    }
    chars
        .find(|next| !matches!(next, ' ' | '\t' | '\r'))
        .is_some_and(is_arithmetic_subscript_operand_start)
}

pub(super) fn is_arithmetic_subscript_operand_start(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '(' | '{')
}
