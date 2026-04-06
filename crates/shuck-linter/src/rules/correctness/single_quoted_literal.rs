use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, ConditionalUnaryOp, DeclClause, DeclOperand, FunctionDef, Position, Redirect,
    SimpleCommand, Span, TextSize, Word, WordPart,
};
use shuck_indexer::Indexer;

use super::syntax::{
    assignment_target_name, effective_command_name, simple_test_operands, static_word_text,
};
use crate::{Checker, Rule, Violation};

pub struct SingleQuotedLiteral;

impl Violation for SingleQuotedLiteral {
    fn rule() -> Rule {
        Rule::SingleQuotedLiteral
    }

    fn message(&self) -> String {
        "shell expansion inside single quotes stays literal".to_owned()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ScanContext<'a> {
    command_name: Option<&'a str>,
    assignment_target: Option<&'a str>,
    variable_set_operand: bool,
}

impl<'a> ScanContext<'a> {
    fn with_assignment_target(self, assignment_target: &'a str) -> Self {
        Self {
            assignment_target: Some(assignment_target),
            ..self
        }
    }

    fn variable_set_operand(self) -> Self {
        Self {
            variable_set_operand: true,
            ..self
        }
    }
}

pub fn single_quoted_literal(checker: &mut Checker) {
    let mut spans = Vec::new();
    collect_commands(
        &checker.ast().commands,
        checker.indexer(),
        checker.source(),
        &mut spans,
    );

    for span in spans {
        checker.report(SingleQuotedLiteral, span);
    }
}

fn collect_commands(commands: &[Command], indexer: &Indexer, source: &str, spans: &mut Vec<Span>) {
    for command in commands {
        collect_command(command, indexer, source, spans);
    }
}

fn collect_command(command: &Command, indexer: &Indexer, source: &str, spans: &mut Vec<Span>) {
    let command_name = effective_command_name(command, source);
    let context = ScanContext {
        command_name: command_name.as_deref(),
        ..ScanContext::default()
    };

    match command {
        Command::Simple(command) => {
            collect_simple_command(command, indexer, source, spans, context)
        }
        Command::Builtin(command) => collect_builtin(command, indexer, source, spans),
        Command::Decl(command) => collect_decl_command(command, indexer, source, spans),
        Command::Pipeline(command) => collect_commands(&command.commands, indexer, source, spans),
        Command::List(CommandList { first, rest, .. }) => {
            collect_command(first, indexer, source, spans);
            for (_, command) in rest {
                collect_command(command, indexer, source, spans);
            }
        }
        Command::Compound(command, redirects) => {
            collect_compound(command, indexer, source, spans);
            collect_redirects(redirects, indexer, source, spans, ScanContext::default());
        }
        Command::Function(FunctionDef { body, .. }) => {
            collect_command(body, indexer, source, spans)
        }
    }
}

fn collect_simple_command(
    command: &SimpleCommand,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    collect_assignments(&command.assignments, indexer, source, spans, context);
    collect_word(&command.name, indexer, source, spans, context);

    let variable_set_operand = simple_command_variable_set_operand(command, source);
    for word in &command.args {
        let context = if variable_set_operand.is_some_and(|operand| std::ptr::eq(word, operand)) {
            context.variable_set_operand()
        } else {
            context
        };
        collect_word(word, indexer, source, spans, context);
    }

    collect_redirects(&command.redirects, indexer, source, spans, context);
}

fn collect_builtin(
    command: &BuiltinCommand,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let context = ScanContext::default();
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignments(&command.assignments, indexer, source, spans, context);
            if let Some(word) = &command.depth {
                collect_word(word, indexer, source, spans, context);
            }
            collect_words(&command.extra_args, indexer, source, spans, context);
            collect_redirects(&command.redirects, indexer, source, spans, context);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignments(&command.assignments, indexer, source, spans, context);
            if let Some(word) = &command.depth {
                collect_word(word, indexer, source, spans, context);
            }
            collect_words(&command.extra_args, indexer, source, spans, context);
            collect_redirects(&command.redirects, indexer, source, spans, context);
        }
        BuiltinCommand::Return(command) => {
            collect_assignments(&command.assignments, indexer, source, spans, context);
            if let Some(word) = &command.code {
                collect_word(word, indexer, source, spans, context);
            }
            collect_words(&command.extra_args, indexer, source, spans, context);
            collect_redirects(&command.redirects, indexer, source, spans, context);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignments(&command.assignments, indexer, source, spans, context);
            if let Some(word) = &command.code {
                collect_word(word, indexer, source, spans, context);
            }
            collect_words(&command.extra_args, indexer, source, spans, context);
            collect_redirects(&command.redirects, indexer, source, spans, context);
        }
    }
}

fn collect_decl_command(
    command: &DeclClause,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let context = ScanContext::default();
    collect_assignments(&command.assignments, indexer, source, spans, context);
    for operand in &command.operands {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                collect_word(word, indexer, source, spans, context);
            }
            DeclOperand::Name(_) => {}
            DeclOperand::Assignment(assignment) => {
                collect_assignment(assignment, indexer, source, spans, context);
            }
        }
    }
    collect_redirects(&command.redirects, indexer, source, spans, context);
}

fn collect_compound(
    command: &CompoundCommand,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_commands(&command.condition, indexer, source, spans);
            collect_commands(&command.then_branch, indexer, source, spans);
            for (condition, body) in &command.elif_branches {
                collect_commands(condition, indexer, source, spans);
                collect_commands(body, indexer, source, spans);
            }
            if let Some(body) = &command.else_branch {
                collect_commands(body, indexer, source, spans);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_words(words, indexer, source, spans, ScanContext::default());
            }
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::ArithmeticFor(command) => {
            collect_commands(&command.body, indexer, source, spans)
        }
        CompoundCommand::While(command) => {
            collect_commands(&command.condition, indexer, source, spans);
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::Until(command) => {
            collect_commands(&command.condition, indexer, source, spans);
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::Case(command) => {
            collect_word(
                &command.word,
                indexer,
                source,
                spans,
                ScanContext::default(),
            );
            for case in &command.cases {
                collect_words(
                    &case.patterns,
                    indexer,
                    source,
                    spans,
                    ScanContext::default(),
                );
                collect_commands(&case.commands, indexer, source, spans);
            }
        }
        CompoundCommand::Select(command) => {
            collect_words(
                &command.words,
                indexer,
                source,
                spans,
                ScanContext::default(),
            );
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_commands(commands, indexer, source, spans);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command(command, indexer, source, spans);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_expr(
                &command.expression,
                indexer,
                source,
                spans,
                ScanContext::default(),
            );
        }
        CompoundCommand::Coproc(command) => collect_command(&command.body, indexer, source, spans),
    }
}

fn collect_assignments(
    assignments: &[Assignment],
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    for assignment in assignments {
        collect_assignment(assignment, indexer, source, spans, context);
    }
}

fn collect_assignment(
    assignment: &Assignment,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    let context = context.with_assignment_target(assignment_target_name(assignment));
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word(word, indexer, source, spans, context),
        AssignmentValue::Array(words) => collect_words(words, indexer, source, spans, context),
    }
}

fn collect_words(
    words: &[Word],
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    for word in words {
        collect_word(word, indexer, source, spans, context);
    }
}

fn collect_word(
    word: &Word,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) if is_single_quoted(indexer, span) => {
                let text = text.as_str(source, span);
                if should_report_single_quoted_literal(text, context) {
                    spans.push(anchor_single_quote_span(indexer, span));
                }
            }
            WordPart::CommandSubstitution(commands)
            | WordPart::ProcessSubstitution { commands, .. } => {
                collect_commands(commands, indexer, source, spans);
            }
            _ => {}
        }
    }
}

fn collect_redirects(
    redirects: &[Redirect],
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    for redirect in redirects {
        collect_word(&redirect.target, indexer, source, spans, context);
    }
}

fn collect_conditional_expr(
    expression: &ConditionalExpr,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
    context: ScanContext<'_>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_expr(&expr.left, indexer, source, spans, context);
            collect_conditional_expr(&expr.right, indexer, source, spans, context);
        }
        ConditionalExpr::Unary(expr) => {
            let context = if expr.op == ConditionalUnaryOp::VariableSet {
                context.variable_set_operand()
            } else {
                context
            };
            collect_conditional_expr(&expr.expr, indexer, source, spans, context);
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_expr(&expr.expr, indexer, source, spans, context);
        }
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => collect_word(word, indexer, source, spans, context),
    }
}

fn simple_command_variable_set_operand<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
    let operands = simple_test_operands(command, source)?;
    (operands.len() == 2 && static_word_text(&operands[0], source).as_deref() == Some("-v"))
        .then(|| &operands[1])
}

fn is_single_quoted(indexer: &Indexer, span: Span) -> bool {
    indexer
        .region_index()
        .single_quoted_range_at(TextSize::new(span.start.offset as u32))
        .is_some()
}

fn should_report_single_quoted_literal(text: &str, context: ScanContext<'_>) -> bool {
    if !contains_sc2016_trigger(text) || context.variable_set_operand {
        return false;
    }

    if context.command_name == Some("sed") {
        return !sed_text_is_exempt(text);
    }

    if context
        .assignment_target
        .is_some_and(assignment_target_is_exempt)
    {
        return false;
    }

    if context.command_name.is_some_and(command_name_is_exempt) {
        return false;
    }

    true
}

fn anchor_single_quote_span(indexer: &Indexer, span: Span) -> Span {
    let offset = TextSize::new(span.start.offset as u32);
    let Some(range) = indexer.region_index().single_quoted_range_at(offset) else {
        return span;
    };

    Span::from_positions(
        position_for_offset(indexer, range.start()),
        position_for_offset(indexer, range.end()),
    )
}

fn position_for_offset(indexer: &Indexer, offset: TextSize) -> Position {
    let line = indexer.line_index().line_number(offset);
    let line_start = indexer
        .line_index()
        .line_start(line)
        .unwrap_or_else(|| TextSize::new(0));

    Position {
        line,
        column: usize::from(offset) - usize::from(line_start) + 1,
        offset: usize::from(offset),
    }
}

fn contains_sc2016_trigger(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] == b'$'
            && matches!(
                bytes[index + 1],
                b'{' | b'(' | b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
            )
        {
            return true;
        }

        if bytes[index] == b'`'
            && bytes.get(index + 1).is_some_and(|next| *next != b'`')
            && bytes[index + 2..].contains(&b'`')
        {
            return true;
        }

        index += 1;
    }

    false
}

fn sed_text_is_exempt(text: &str) -> bool {
    let bytes = text.as_bytes();

    for index in 0..bytes.len().saturating_sub(1) {
        if bytes[index] != b'$' {
            continue;
        }

        let next = bytes[index + 1];
        if !matches!(next, b'{' | b'd' | b'p' | b's' | b'a' | b'i' | b'c') {
            continue;
        }

        let following = bytes.get(index + 2).copied();
        if following.is_none_or(|byte| !byte.is_ascii_alphabetic()) {
            return true;
        }
    }

    false
}

fn assignment_target_is_exempt(target: &str) -> bool {
    matches!(target, "PS1" | "PS2" | "PS3" | "PS4" | "PROMPT_COMMAND")
}

fn command_name_is_exempt(command_name: &str) -> bool {
    matches!(
        command_name,
        "trap"
            | "sh"
            | "bash"
            | "ksh"
            | "zsh"
            | "ssh"
            | "eval"
            | "xprop"
            | "alias"
            | "sudo"
            | "doas"
            | "run0"
            | "docker"
            | "podman"
            | "oc"
            | "dpkg-query"
            | "jq"
            | "rename"
            | "rg"
            | "unset"
            | "git filter-branch"
            | "mumps -run %XCMD"
            | "mumps -run LOOP%XCMD"
    ) || command_name.ends_with("awk")
        || command_name.starts_with("perl")
}

#[cfg(test)]
mod tests {
    use super::{
        assignment_target_is_exempt, command_name_is_exempt, contains_sc2016_trigger,
        sed_text_is_exempt,
    };
    use crate::test::test_snippet;
    use crate::{Diagnostic, LinterSettings, Rule};

    fn c005(source: &str) -> usize {
        c005_diagnostics(source).len()
    }

    fn c005_diagnostics(source: &str) -> Vec<Diagnostic> {
        test_snippet(source, &LinterSettings::for_rule(Rule::SingleQuotedLiteral))
    }

    #[test]
    fn detects_sc2016_variable_like_sequences_and_backticks() {
        assert!(contains_sc2016_trigger("$HOME"));
        assert!(contains_sc2016_trigger("${name:-default}"));
        assert!(contains_sc2016_trigger("$(pwd)"));
        assert!(contains_sc2016_trigger("$1"));
        assert!(contains_sc2016_trigger("`pwd`"));
    }

    #[test]
    fn ignores_shellcheck_exempt_special_parameter_sequences() {
        for text in ["$$", "$?", "$#", "$@", "$*", "$!", "$-", "$", "hello world"] {
            assert!(!contains_sc2016_trigger(text), "{text}");
        }
    }

    #[test]
    fn recognizes_sed_exemptions() {
        assert!(sed_text_is_exempt("$p"));
        assert!(sed_text_is_exempt("${/lol/d}"));
        assert!(!sed_text_is_exempt("$pattern"));
    }

    #[test]
    fn recognizes_shellcheck_style_command_and_assignment_exemptions() {
        for command_name in [
            "awk",
            "gawk",
            "perl",
            "perl5.38",
            "trap",
            "alias",
            "jq",
            "git filter-branch",
        ] {
            assert!(command_name_is_exempt(command_name), "{command_name}");
        }

        for target in ["PS1", "PS2", "PS3", "PS4", "PROMPT_COMMAND"] {
            assert!(assignment_target_is_exempt(target), "{target}");
        }

        assert!(!command_name_is_exempt("echo"));
        assert!(!assignment_target_is_exempt("HOME"));
    }

    #[test]
    fn rule_detects_backticks_and_respects_exemptions() {
        assert_eq!(c005("echo '`pwd`'\n"), 1);
        assert_eq!(c005("echo '$@'\n"), 0);
        assert_eq!(c005("awk '{print $1}'\n"), 0);
        assert_eq!(c005("PS1='$PWD \\\\$ '\n"), 0);
        assert_eq!(c005("command jq '$__loc__'\n"), 0);
        assert_eq!(c005("sed -n '$p'\n"), 0);
        assert_eq!(c005("sed -n '$pattern'\n"), 1);
    }

    #[test]
    fn corpus_regression_teamcity_awk_is_exempt() {
        assert_eq!(c005("awk '{print $5}' || :\n"), 0);
    }

    #[test]
    fn corpus_regression_alias_wrapper_is_exempt() {
        assert_eq!(c005("alias hosts='sudo $EDITOR /etc/hosts'\n"), 0);
    }

    #[test]
    fn corpus_regression_special_parameters_are_exempt() {
        assert_eq!(c005("SHOBJ_LDFLAGS='-shared -Wl,-h,$@'\n"), 0);
        assert_eq!(c005("SHOBJ_LDFLAGS='-G -dy -z text -i -h $@'\n"), 0);
    }

    #[test]
    fn corpus_regression_backticks_are_reported() {
        let diagnostics = c005_diagnostics("SHOBJ_ARCHFLAGS='-arch_only `/usr/bin/arch`'\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 17);
    }

    #[test]
    fn corpus_regression_openvpn_sample_anchors_on_opening_quote() {
        let diagnostics = c005_diagnostics(
            "if ! grep -q sbin <<< \"$PATH\"; then\n\techo '$PATH does not include sbin. Try using \"su -\" instead of \"su\".'\nfi\n",
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 7);
    }

    #[test]
    fn diagnostic_span_covers_the_full_single_quoted_region() {
        let diagnostics = c005_diagnostics("echo '$HOME'\n");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.end.line, 1);
        assert_eq!(diagnostics[0].span.end.column, 13);
    }

    #[test]
    fn corpus_regression_omarchy_sample_anchors_on_opening_quote() {
        let diagnostics = c005_diagnostics(
            "  sed -i '/bindd = SUPER, RETURN, Terminal, exec, \\$terminal/ s|$| --working-directory=$(omarchy-cmd-terminal-cwd)|' ~/.config/hypr/bindings.conf\n",
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 10);
    }

    #[test]
    fn variable_set_operand_helper_does_not_panic_on_incomplete_operands() {
        assert_eq!(c005("test -v\n"), 0);
        assert_eq!(c005("test -v name\n"), 0);
    }
}
