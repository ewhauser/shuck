use shuck_ast::{
    BourneParameterExpansion, Command, CompoundCommand, ConditionalBinaryExpr, ConditionalBinaryOp,
    ConditionalExpr, ConditionalUnaryExpr, ConditionalUnaryOp, File, HeredocBody, HeredocBodyPart,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, SourceText, Stmt, StmtSeq, VarRef,
    Word, WordPart, WordPartNode, ZshExpansionOperation, ZshExpansionTarget,
};

use crate::command::render_var_ref_to_buf;
use crate::visit::{self, AstVisitorMut};
use crate::word::parameter_defaulting_operator;

const PAREN_CLEANUP: &str = "paren-cleanup";
const ARITHMETIC_VARS: &str = "arithmetic-vars";
const CONDITIONALS: &str = "conditionals";
const NESTED_SUBSHELLS: &str = "nested-subshells";
const QUOTE_TIGHTENING: &str = "quote-tightening";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimplifyReport {
    pub applied: Vec<RewriteApplication>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteApplication {
    pub name: &'static str,
    pub changes: usize,
}

#[derive(Default)]
struct RewriteCounts {
    paren_cleanup: std::cell::Cell<usize>,
    arithmetic_vars: std::cell::Cell<usize>,
    conditionals: std::cell::Cell<usize>,
    nested_subshells: std::cell::Cell<usize>,
    quote_tightening: std::cell::Cell<usize>,
}

impl RewriteCounts {
    fn add_paren_cleanup(&self, changes: usize) {
        self.paren_cleanup.set(self.paren_cleanup.get() + changes);
    }

    fn add_arithmetic_vars(&self, changes: usize) {
        self.arithmetic_vars
            .set(self.arithmetic_vars.get() + changes);
    }

    fn add_conditionals(&self, changes: usize) {
        self.conditionals.set(self.conditionals.get() + changes);
    }

    fn add_nested_subshells(&self, changes: usize) {
        self.nested_subshells
            .set(self.nested_subshells.get() + changes);
    }

    fn add_quote_tightening(&self, changes: usize) {
        self.quote_tightening
            .set(self.quote_tightening.get() + changes);
    }

    fn into_report(self) -> SimplifyReport {
        let mut applied = Vec::new();
        for (name, changes) in [
            (PAREN_CLEANUP, self.paren_cleanup.get()),
            (ARITHMETIC_VARS, self.arithmetic_vars.get()),
            (CONDITIONALS, self.conditionals.get()),
            (NESTED_SUBSHELLS, self.nested_subshells.get()),
            (QUOTE_TIGHTENING, self.quote_tightening.get()),
        ] {
            if changes > 0 {
                applied.push(RewriteApplication { name, changes });
            }
        }
        SimplifyReport { applied }
    }
}

pub fn simplify_file(file: &mut File, source: &str) -> SimplifyReport {
    let counts = RewriteCounts::default();
    {
        let mut visitor = SimplifyVisitor {
            source,
            counts: &counts,
            mode: SimplifyTraversalMode::Full,
        };
        visitor.visit_file(file);
    }
    counts.into_report()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimplifyTraversalMode {
    Full,
    HeredocCommandSubstitution,
}

struct SimplifyVisitor<'source, 'counts> {
    source: &'source str,
    counts: &'counts RewriteCounts,
    mode: SimplifyTraversalMode,
}

impl SimplifyVisitor<'_, '_> {
    fn with_mode(
        &mut self,
        mode: SimplifyTraversalMode,
        visit: impl FnOnce(&mut Self) -> usize,
    ) -> usize {
        let previous = self.mode;
        self.mode = mode;
        let changes = visit(self);
        self.mode = previous;
        changes
    }
}

impl AstVisitorMut for SimplifyVisitor<'_, '_> {
    fn enter_stmt(&mut self, stmt: &mut Stmt) -> usize {
        if self.mode == SimplifyTraversalMode::HeredocCommandSubstitution {
            return 0;
        }

        let mut changes = 0;

        if let Command::Compound(CompoundCommand::Conditional(conditional)) = &mut stmt.command {
            let count = simplify_conditional_expr(&mut conditional.expression, self.source);
            self.counts.add_conditionals(count);
            changes += count;
        }

        if let Command::Compound(CompoundCommand::Subshell(commands)) = &mut stmt.command
            && !stmt.negated
            && stmt.redirects.is_empty()
            && stmt.terminator.is_none()
        {
            let count = collapse_nested_subshell_sequence(commands);
            self.counts.add_nested_subshells(count);
            changes += count;
        }

        changes
    }

    fn visit_word(&mut self, word: &mut Word) -> usize {
        let mut changes = if self.mode == SimplifyTraversalMode::Full {
            visit::walk_word_surface_source_texts_mut(self, word)
        } else {
            0
        };

        let nested_subshells = collapse_nested_subshells_in_word(word);
        self.counts.add_nested_subshells(nested_subshells);
        changes += nested_subshells;

        let quote_tightening = tighten_literal_quotes(word, self.source);
        self.counts.add_quote_tightening(quote_tightening);
        changes += quote_tightening;

        changes
    }

    fn visit_heredoc_body(&mut self, body: &mut HeredocBody) -> usize {
        let mut count = 0;

        for part in &mut body.parts {
            count += match &mut part.kind {
                HeredocBodyPart::CommandSubstitution {
                    body: command_body, ..
                } => self.with_mode(
                    SimplifyTraversalMode::HeredocCommandSubstitution,
                    |visitor| visitor.visit_stmt_seq(command_body),
                ),
                HeredocBodyPart::ArithmeticExpansion { expression, .. } => {
                    self.visit_source_text(expression)
                }
                HeredocBodyPart::Parameter(parameter) => self.visit_parameter_expansion(parameter),
                HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => 0,
            };
        }

        if count > 0 {
            body.source_backed = false;
        }

        count
    }

    fn visit_parameter_expansion(&mut self, parameter: &mut ParameterExpansion) -> usize {
        let count = visit::walk_parameter_expansion_surface_source_texts_mut(self, parameter);
        if count > 0 {
            parameter.raw_body = SourceText::cooked(
                parameter.raw_body.span(),
                render_parameter_raw_body(parameter, self.source),
            );
        }
        count
    }

    fn visit_source_text(&mut self, text: &mut SourceText) -> usize {
        if self.mode == SimplifyTraversalMode::HeredocCommandSubstitution {
            return 0;
        }

        let paren_cleanup = transform_source_text(text, self.source, strip_single_outer_parens);
        self.counts.add_paren_cleanup(paren_cleanup);

        let arithmetic_vars =
            transform_source_text(text, self.source, simplify_arithmetic_variables_text);
        self.counts.add_arithmetic_vars(arithmetic_vars);

        paren_cleanup + arithmetic_vars
    }
}

fn collapse_nested_subshell_sequence(commands: &mut StmtSeq) -> usize {
    let mut changes = 0;
    while commands.leading_comments.is_empty()
        && commands.trailing_comments.is_empty()
        && commands.len() == 1
    {
        let Some(Stmt {
            leading_comments,
            command: Command::Compound(CompoundCommand::Subshell(inner)),
            negated: false,
            redirects,
            terminator: None,
            inline_comment: None,
            ..
        }) = commands.first()
        else {
            break;
        };
        if !leading_comments.is_empty() || !redirects.is_empty() {
            break;
        }
        *commands = inner.clone();
        changes += 1;
    }
    changes
}

fn collapse_nested_subshells_in_word(word: &mut Word) -> usize {
    word.parts
        .iter_mut()
        .map(|part| match &mut part.kind {
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => collapse_nested_subshell_sequence(body),
            _ => 0,
        })
        .sum()
}

fn render_parameter_raw_body(parameter: &ParameterExpansion, source: &str) -> String {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => {
            render_bourne_parameter_raw_body(syntax, source)
        }
        ParameterExpansionSyntax::Zsh(syntax) => render_zsh_parameter_raw_body(syntax, source),
    }
}

fn render_bourne_parameter_raw_body(syntax: &BourneParameterExpansion, source: &str) -> String {
    match syntax {
        BourneParameterExpansion::Access { reference } => render_var_ref_syntax(reference, source),
        BourneParameterExpansion::Length { reference } => {
            format!("#{}", render_var_ref_syntax(reference, source))
        }
        BourneParameterExpansion::Indices { reference } => {
            format!("!{}", render_var_ref_syntax(reference, source))
        }
        BourneParameterExpansion::Indirect {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            let mut rendered = format!("!{}", render_var_ref_syntax(reference, source));
            if let Some(operator) = operator {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(parameter_defaulting_operator(operator));
                if let Some(operand) = operand {
                    rendered.push_str(operand.slice(source));
                }
            }
            rendered
        }
        BourneParameterExpansion::PrefixMatch { prefix, kind } => {
            format!("!{}{}", prefix, kind.as_char())
        }
        BourneParameterExpansion::Slice {
            reference,
            offset,
            length,
            ..
        } => {
            let mut rendered = format!(
                "{}:{}",
                render_var_ref_syntax(reference, source),
                offset.slice(source)
            );
            if let Some(length) = length {
                rendered.push(':');
                rendered.push_str(length.slice(source));
            }
            rendered
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            let mut rendered = render_var_ref_syntax(reference, source);
            match operator.as_ref() {
                ParameterOp::UseDefault
                | ParameterOp::AssignDefault
                | ParameterOp::UseReplacement
                | ParameterOp::Error => {
                    if *colon_variant {
                        rendered.push(':');
                    }
                    rendered.push_str(parameter_defaulting_operator(operator));
                    if let Some(operand) = operand {
                        rendered.push_str(operand.slice(source));
                    }
                }
                ParameterOp::RemovePrefixShort { pattern } => {
                    rendered.push('#');
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::RemovePrefixLong { pattern } => {
                    rendered.push_str("##");
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::RemoveSuffixShort { pattern } => {
                    rendered.push('%');
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::RemoveSuffixLong { pattern } => {
                    rendered.push_str("%%");
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::ReplaceFirst {
                    pattern,
                    replacement,
                    ..
                } => {
                    rendered.push('/');
                    pattern.render_syntax_to_buf(source, &mut rendered);
                    rendered.push('/');
                    rendered.push_str(replacement.slice(source));
                }
                ParameterOp::ReplaceAll {
                    pattern,
                    replacement,
                    ..
                } => {
                    rendered.push_str("//");
                    pattern.render_syntax_to_buf(source, &mut rendered);
                    rendered.push('/');
                    rendered.push_str(replacement.slice(source));
                }
                ParameterOp::UpperFirst => rendered.push('^'),
                ParameterOp::UpperAll => rendered.push_str("^^"),
                ParameterOp::LowerFirst => rendered.push(','),
                ParameterOp::LowerAll => rendered.push_str(",,"),
            }
            rendered
        }
        BourneParameterExpansion::Transformation {
            reference,
            operator,
        } => format!("{}@{}", render_var_ref_syntax(reference, source), operator),
    }
}

fn render_zsh_parameter_raw_body(
    syntax: &shuck_ast::ZshParameterExpansion,
    source: &str,
) -> String {
    let mut rendered = String::new();
    let mut modifier_index = 0usize;

    while modifier_index < syntax.modifiers.len() {
        let group_span = syntax.modifiers[modifier_index].span;
        rendered.push('(');
        while modifier_index < syntax.modifiers.len()
            && syntax.modifiers[modifier_index].span == group_span
        {
            let modifier = &syntax.modifiers[modifier_index];
            rendered.push(modifier.name);
            if let Some(delimiter) = modifier.argument_delimiter {
                rendered.push(delimiter);
                if let Some(argument) = &modifier.argument {
                    rendered.push_str(argument.slice(source));
                }
                rendered.push(delimiter);
            }
            modifier_index += 1;
        }
        rendered.push(')');
    }

    match &syntax.target {
        ZshExpansionTarget::Reference(reference) => {
            rendered.push_str(&render_var_ref_syntax(reference, source));
        }
        ZshExpansionTarget::Word(word) => {
            rendered.push_str(&word.render(source));
        }
        ZshExpansionTarget::Nested(parameter) => {
            rendered.push_str("${");
            rendered.push_str(&render_parameter_raw_body(parameter, source));
            rendered.push('}');
        }
        ZshExpansionTarget::Empty => {}
    }

    if let Some(operation) = &syntax.operation {
        match operation {
            ZshExpansionOperation::PatternOperation { operand, .. } => {
                rendered.push_str(":#");
                rendered.push_str(operand.slice(source));
            }
            ZshExpansionOperation::Defaulting {
                kind,
                operand,
                colon_variant,
                ..
            } => {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(match kind {
                    shuck_ast::ZshDefaultingOp::UseDefault => "-",
                    shuck_ast::ZshDefaultingOp::AssignDefault => "=",
                    shuck_ast::ZshDefaultingOp::UseReplacement => "+",
                    shuck_ast::ZshDefaultingOp::Error => "?",
                });
                rendered.push_str(operand.slice(source));
            }
            ZshExpansionOperation::TrimOperation { kind, operand, .. } => {
                rendered.push_str(match kind {
                    shuck_ast::ZshTrimOp::RemovePrefixShort => "#",
                    shuck_ast::ZshTrimOp::RemovePrefixLong => "##",
                    shuck_ast::ZshTrimOp::RemoveSuffixShort => "%",
                    shuck_ast::ZshTrimOp::RemoveSuffixLong => "%%",
                });
                rendered.push_str(operand.slice(source));
            }
            ZshExpansionOperation::ReplacementOperation {
                kind,
                pattern,
                replacement,
                ..
            } => {
                rendered.push_str(match kind {
                    shuck_ast::ZshReplacementOp::ReplaceFirst => "/",
                    shuck_ast::ZshReplacementOp::ReplaceAll => "//",
                    shuck_ast::ZshReplacementOp::ReplacePrefix => "/#",
                    shuck_ast::ZshReplacementOp::ReplaceSuffix => "/%",
                });
                rendered.push_str(pattern.slice(source));
                if let Some(replacement) = replacement {
                    rendered.push('/');
                    rendered.push_str(replacement.slice(source));
                }
            }
            ZshExpansionOperation::Slice { offset, length, .. } => {
                rendered.push(':');
                rendered.push_str(offset.slice(source));
                if let Some(length) = length {
                    rendered.push(':');
                    rendered.push_str(length.slice(source));
                }
            }
            ZshExpansionOperation::Unknown { text, .. } => rendered.push_str(text.slice(source)),
        }
    }

    rendered
}

fn render_var_ref_syntax(reference: &VarRef, source: &str) -> String {
    let mut rendered = String::new();
    render_var_ref_to_buf(reference, source, &mut rendered);
    rendered
}

fn transform_source_text(
    text: &mut SourceText,
    source: &str,
    transform: impl FnOnce(&str) -> Option<String>,
) -> usize {
    let current = text.slice(source);
    let Some(next) = transform(current) else {
        return 0;
    };
    if next == current {
        return 0;
    }
    *text = SourceText::cooked(text.span(), next);
    1
}

fn strip_single_outer_parens(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
        return None;
    }

    let mut depth = 0usize;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index + ch.len_utf8() != trimmed.len() {
                    return None;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return None;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    (!inner.is_empty() && !inner.contains('\n')).then(|| inner.to_string())
}

fn simplify_arithmetic_variables_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if let Some(name) = trimmed.strip_prefix('$')
        && !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '[' | ']'))
        && !name.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    let mut output = String::with_capacity(text.len());
    let mut changed = false;
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] != '$' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        if index + 1 >= chars.len() {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        if chars[index + 1] == '{' {
            let mut end = index + 2;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            if end < chars.len() {
                let inner: String = chars[index + 2..end].iter().collect();
                if arithmetic_parameter_is_safe(&inner) {
                    output.push_str(&inner);
                    changed = true;
                    index = end + 1;
                    continue;
                }
            }
        } else if chars[index + 1].is_ascii_alphabetic() || chars[index + 1] == '_' {
            let mut end = index + 2;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || chars[end] == '_'
                    || chars[end] == '['
                    || chars[end] == ']')
            {
                end += 1;
            }
            let ident: String = chars[index + 1..end].iter().collect();
            output.push_str(&ident);
            changed = true;
            index = end;
            continue;
        }

        output.push(chars[index]);
        index += 1;
    }

    changed.then_some(output)
}

fn arithmetic_parameter_is_safe(text: &str) -> bool {
    !text.is_empty()
        && !matches!(text.chars().next(), Some('!' | '#'))
        && !text.chars().all(|ch| ch.is_ascii_digit())
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '[' | ']'))
}

fn simplify_conditional_expr(expression: &mut ConditionalExpr, source: &str) -> usize {
    let mut changes = match expression {
        ConditionalExpr::Binary(expr) => {
            let mut count = simplify_conditional_expr(&mut expr.left, source)
                + simplify_conditional_expr(&mut expr.right, source);

            if expr.op == ConditionalBinaryOp::PatternEqShort {
                expr.op = ConditionalBinaryOp::PatternEq;
                count += 1;
            }

            count += strip_redundant_conditional_quotes(&mut expr.left, source);
            count += strip_redundant_conditional_quotes(&mut expr.right, source);
            count
        }
        ConditionalExpr::Unary(expr) => {
            let mut count = simplify_conditional_expr(&mut expr.expr, source);
            count += strip_redundant_conditional_quotes(&mut expr.expr, source);
            count
        }
        ConditionalExpr::Parenthesized(expr) => simplify_conditional_expr(&mut expr.expr, source),
        ConditionalExpr::Word(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => 0,
    };

    loop {
        let Some(next) = simplify_conditional_node(expression) else {
            break;
        };
        *expression = next;
        changes += 1;
    }

    changes
}

fn simplify_conditional_node(expression: &ConditionalExpr) -> Option<ConditionalExpr> {
    match expression {
        ConditionalExpr::Unary(expr) if expr.op == ConditionalUnaryOp::Not => {
            match expr.expr.as_ref() {
                ConditionalExpr::Unary(inner) if inner.op == ConditionalUnaryOp::Not => {
                    Some((*inner.expr).clone())
                }
                ConditionalExpr::Unary(inner) if inner.op == ConditionalUnaryOp::NonEmptyString => {
                    Some(ConditionalExpr::Unary(ConditionalUnaryExpr {
                        op: ConditionalUnaryOp::EmptyString,
                        op_span: inner.op_span,
                        expr: inner.expr.clone(),
                    }))
                }
                ConditionalExpr::Unary(inner) if inner.op == ConditionalUnaryOp::EmptyString => {
                    Some(ConditionalExpr::Unary(ConditionalUnaryExpr {
                        op: ConditionalUnaryOp::NonEmptyString,
                        op_span: inner.op_span,
                        expr: inner.expr.clone(),
                    }))
                }
                ConditionalExpr::Binary(inner) => invert_conditional_binary(inner),
                ConditionalExpr::Parenthesized(inner) => Some((*inner.expr).clone()),
                _ => None,
            }
        }
        ConditionalExpr::Parenthesized(expr)
            if matches!(
                expr.expr.as_ref(),
                ConditionalExpr::Unary(_)
                    | ConditionalExpr::Word(_)
                    | ConditionalExpr::Pattern(_)
                    | ConditionalExpr::Regex(_)
                    | ConditionalExpr::VarRef(_)
            ) =>
        {
            Some((*expr.expr).clone())
        }
        _ => None,
    }
}

fn invert_conditional_binary(expression: &ConditionalBinaryExpr) -> Option<ConditionalExpr> {
    let op = match expression.op {
        ConditionalBinaryOp::PatternEqShort | ConditionalBinaryOp::PatternEq => {
            ConditionalBinaryOp::PatternNe
        }
        ConditionalBinaryOp::PatternNe => ConditionalBinaryOp::PatternEq,
        _ => return None,
    };
    Some(ConditionalExpr::Binary(ConditionalBinaryExpr {
        left: expression.left.clone(),
        op,
        op_span: expression.op_span,
        right: expression.right.clone(),
    }))
}

fn strip_redundant_conditional_quotes(expression: &mut ConditionalExpr, source: &str) -> usize {
    let ConditionalExpr::Word(word) = expression else {
        return 0;
    };

    if !word_is_redundantly_quoted_variable(word) {
        return 0;
    }

    let Some(WordPart::DoubleQuoted { parts, .. }) = word.parts.first().map(|part| &part.kind)
    else {
        return 0;
    };
    let parts = parts.clone();
    *word = Word {
        parts,
        span: word.span,
        brace_syntax: Vec::new(),
    };
    let _ = source;
    1
}

fn word_is_redundantly_quoted_variable(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::DoubleQuoted { parts, dollar: false },
            ..
        }] if matches!(parts.as_slice(), [WordPartNode { kind: WordPart::Variable(_), .. }])
    )
}

fn tighten_literal_quotes(word: &mut Word, source: &str) -> usize {
    let Some(part) = word.parts.first_mut() else {
        return 0;
    };

    let WordPart::DoubleQuoted { parts, dollar } = &part.kind else {
        return 0;
    };
    if *dollar {
        return 0;
    }

    let Some(literal) = literal_text_from_double_quoted(parts, source) else {
        return 0;
    };
    if literal.is_empty() || literal.contains('\'') || literal.contains('\n') {
        return 0;
    }

    *word = Word {
        parts: vec![WordPartNode::new(
            WordPart::SingleQuoted {
                value: SourceText::cooked(word.span, literal),
                dollar: false,
            },
            word.span,
        )],
        span: word.span,
        brace_syntax: Vec::new(),
    };
    1
}

fn literal_text_from_double_quoted(parts: &[WordPartNode], source: &str) -> Option<String> {
    let mut value = String::new();
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let raw = text.as_str(source, part.span);
                if text.is_source_backed() && raw.contains('\\') {
                    return None;
                }
                value.push_str(raw);
            }
            _ => return None,
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_parser::parser::Parser;

    use crate::options::ShellFormatOptions;

    use super::*;

    fn format_with_simplify_at(source: &str, path: &str) -> String {
        match crate::format_source(
            source,
            Some(Path::new(path)),
            &ShellFormatOptions::default().with_simplify(true),
        )
        .unwrap()
        {
            crate::FormattedSource::Unchanged => source.to_string(),
            crate::FormattedSource::Formatted(formatted) => formatted,
        }
    }

    fn format_with_simplify(source: &str) -> String {
        format_with_simplify_at(source, "test.sh")
    }

    fn format_bash_with_simplify(source: &str) -> String {
        format_with_simplify_at(source, "test.bash")
    }

    #[test]
    fn paren_cleanup_simplifies_index_fragments() {
        assert_eq!(
            format_with_simplify("echo ${foo[(1)]}\n"),
            "echo ${foo[1]}\n"
        );
    }

    #[test]
    fn paren_cleanup_skips_dynamic_indexes() {
        assert_eq!(
            format_with_simplify("echo ${foo[$bar]}\n"),
            "echo ${foo[$bar]}\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_unwraps_simple_parameters() {
        assert_eq!(
            format_with_simplify("echo $(( $a + ${b} ))\n"),
            "echo $((a + b))\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_skips_special_parameters() {
        assert_eq!(
            format_with_simplify("echo $(( ${!a} + ${#b} ))\n"),
            "echo $((${!a} + ${#b}))\n"
        );
    }

    #[test]
    fn conditional_rewrite_normalizes_not_and_short_equals() {
        assert_eq!(
            format_bash_with_simplify("[[ ! -n \"$foo\" ]]\n"),
            "[[ -z $foo ]]\n"
        );
        assert_eq!(
            format_bash_with_simplify("[[ foo = bar ]]\n"),
            "[[ foo == bar ]]\n"
        );
    }

    #[test]
    fn quote_tightening_rewrites_simple_literal_quotes() {
        assert_eq!(format_with_simplify("echo \"fo\\$o\"\n"), "echo 'fo$o'\n");
    }

    #[test]
    fn quote_tightening_skips_mixed_expansions() {
        assert_eq!(
            format_with_simplify("echo \"$foo bar\"\n"),
            "echo \"$foo bar\"\n"
        );
    }

    #[test]
    fn quote_tightening_rewrites_command_substitutions_inside_heredoc_bodies() {
        assert_eq!(
            format_with_simplify("cat <<EOF\n$(printf \"%s\" \"fo\\$o\")\nEOF\n"),
            "cat <<EOF\n$(printf '%s' 'fo$o')\nEOF\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_updates_expanding_heredoc_bodies() {
        assert_eq!(
            format_with_simplify("cat <<EOF\n$(( $a + ${b} ))\nEOF\n"),
            "cat <<EOF\n$((a + b))\nEOF\n"
        );
    }

    #[test]
    fn simplify_report_tracks_applied_rewrites() {
        let parsed = Parser::new("echo $(( $a + ${b} ))\n").parse().unwrap();
        let mut file = parsed.file.clone();
        let report = simplify_file(&mut file, "echo $(( $a + ${b} ))\n");
        let total_changes: usize = report.applied.iter().map(|entry| entry.changes).sum();

        assert_eq!(total_changes, 1);
        assert_eq!(report.applied.len(), 1);
        assert_eq!(report.applied[0].name, "arithmetic-vars");
        assert_eq!(report.applied[0].changes, 1);
    }
}
