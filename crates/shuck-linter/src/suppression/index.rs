use rustc_hash::FxHashMap;
use shuck_ast::{
    ArrayElem, Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, Pattern, PatternPart, Redirect, Script, Span,
    Subscript, TextSize, Word, WordPart, WordPartNode,
};
use shuck_parser::parser::Parser;

use crate::Rule;
use crate::rules::common::query;

use super::{SuppressionAction, SuppressionDirective, SuppressionSource};

/// Per-file suppression index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuppressionIndex {
    by_rule: FxHashMap<Rule, RuleSuppressionIndex>,
}

impl SuppressionIndex {
    /// Build from parsed directives.
    pub fn new(
        directives: &[SuppressionDirective],
        script: &Script,
        source: &str,
        first_stmt_line: u32,
    ) -> Self {
        let mut ordered = directives.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|directive| {
            (
                directive.line,
                directive.range.start().to_u32(),
                directive.range.end().to_u32(),
            )
        });

        let mut by_rule = FxHashMap::default();
        for directive in ordered {
            for &rule in &directive.codes {
                let index = by_rule
                    .entry(rule)
                    .or_insert_with(RuleSuppressionIndex::default);
                match directive.action {
                    SuppressionAction::DisableFile => index.whole_file = true,
                    SuppressionAction::Disable
                        if directive.source == SuppressionSource::ShellCheck =>
                    {
                        if directive.line < first_stmt_line {
                            index.whole_file = true;
                        } else if let Some(range) =
                            next_command_range(script, source, directive.range.end())
                        {
                            index.ranges.push(range);
                        }
                    }
                    SuppressionAction::Disable => index.events.push(RegionEvent {
                        line: directive.line,
                        suppressed: true,
                    }),
                    SuppressionAction::Enable => index.events.push(RegionEvent {
                        line: directive.line,
                        suppressed: false,
                    }),
                }
            }
        }

        for index in by_rule.values_mut() {
            index
                .ranges
                .sort_unstable_by_key(|range| (range.start_line, range.end_line));
            merge_overlapping_ranges(&mut index.ranges);
        }

        Self { by_rule }
    }

    /// Check if `rule` is suppressed on `line`.
    pub fn is_suppressed(&self, rule: Rule, line: u32) -> bool {
        self.by_rule
            .get(&rule)
            .is_some_and(|index| index.is_suppressed(line))
    }
}

/// First command line in the file, if any.
pub fn first_statement_line(script: &Script) -> Option<u32> {
    script
        .commands
        .iter()
        .filter_map(|command| u32::try_from(command_span(command).start.line).ok())
        .min()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuleSuppressionIndex {
    whole_file: bool,
    events: Vec<RegionEvent>,
    ranges: Vec<LineRange>,
}

impl RuleSuppressionIndex {
    fn is_suppressed(&self, line: u32) -> bool {
        if self.whole_file {
            return true;
        }

        let candidate = self
            .ranges
            .partition_point(|range| range.start_line <= line);
        if let Some(range) = candidate
            .checked_sub(1)
            .and_then(|index| self.ranges.get(index))
            && line <= range.end_line
        {
            return true;
        }

        self.events
            .partition_point(|event| event.line <= line)
            .checked_sub(1)
            .and_then(|index| self.events.get(index))
            .is_some_and(|event| event.suppressed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionEvent {
    line: u32,
    suppressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineRange {
    start_line: u32,
    end_line: u32,
}

fn merge_overlapping_ranges(ranges: &mut Vec<LineRange>) {
    if ranges.len() < 2 {
        return;
    }

    let mut merged = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];

    for range in ranges.iter().copied().skip(1) {
        if range.start_line <= current.end_line {
            current.end_line = current.end_line.max(range.end_line);
        } else {
            merged.push(current);
            current = range;
        }
    }
    merged.push(current);
    *ranges = merged;
}

fn next_command_range(script: &Script, source: &str, offset: TextSize) -> Option<LineRange> {
    let mut next = None;
    for command in &script.commands {
        walk_command(command, source, &mut |span| {
            consider_command(span, offset, &mut next)
        });
    }

    next.and_then(line_range)
}

fn consider_command(span: Span, offset: TextSize, next: &mut Option<Span>) {
    if span.start.line == 0 || span.end.line == 0 {
        return;
    }

    let start = TextSize::new(span.start.offset as u32);
    if start <= offset {
        return;
    }

    if next
        .as_ref()
        .is_none_or(|current| span.start.offset < current.start.offset)
    {
        *next = Some(span);
    }
}

fn line_range(span: Span) -> Option<LineRange> {
    let start_line = u32::try_from(span.start.line).ok()?;
    let mut end_line = u32::try_from(span.end.line).ok()?;
    if span.end.offset > span.start.offset && span.end.column == 1 {
        end_line = end_line.saturating_sub(1);
    }

    Some(LineRange {
        start_line,
        end_line: end_line.max(start_line),
    })
}

fn walk_commands<F>(commands: &[Command], source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for command in commands {
        walk_command(command, source, visit);
    }
}

fn walk_command<F>(command: &Command, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    visit(command_span(command));

    match command {
        Command::Simple(command) => {
            walk_assignments(&command.assignments, source, visit);
            walk_word(&command.name, source, visit);
            walk_words(&command.args, source, visit);
            walk_redirects(&command.redirects, source, visit);
        }
        Command::Builtin(BuiltinCommand::Break(command)) => {
            walk_assignments(&command.assignments, source, visit);
            if let Some(word) = &command.depth {
                walk_word(word, source, visit);
            }
            walk_words(&command.extra_args, source, visit);
            walk_redirects(&command.redirects, source, visit);
        }
        Command::Builtin(BuiltinCommand::Continue(command)) => {
            walk_assignments(&command.assignments, source, visit);
            if let Some(word) = &command.depth {
                walk_word(word, source, visit);
            }
            walk_words(&command.extra_args, source, visit);
            walk_redirects(&command.redirects, source, visit);
        }
        Command::Builtin(BuiltinCommand::Return(command)) => {
            walk_assignments(&command.assignments, source, visit);
            if let Some(word) = &command.code {
                walk_word(word, source, visit);
            }
            walk_words(&command.extra_args, source, visit);
            walk_redirects(&command.redirects, source, visit);
        }
        Command::Builtin(BuiltinCommand::Exit(command)) => {
            walk_assignments(&command.assignments, source, visit);
            if let Some(word) = &command.code {
                walk_word(word, source, visit);
            }
            walk_words(&command.extra_args, source, visit);
            walk_redirects(&command.redirects, source, visit);
        }
        Command::Decl(command) => {
            walk_assignments(&command.assignments, source, visit);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        walk_word(word, source, visit)
                    }
                    DeclOperand::Name(reference) => {
                        walk_var_ref_subscript(reference, source, visit)
                    }
                    DeclOperand::Assignment(assignment) => {
                        walk_assignment(assignment, source, visit)
                    }
                }
            }
            walk_redirects(&command.redirects, source, visit);
        }
        Command::Pipeline(command) => walk_commands(&command.commands, source, visit),
        Command::List(command) => {
            walk_command(command.first.as_ref(), source, visit);
            for item in &command.rest {
                walk_command(&item.command, source, visit);
            }
        }
        Command::Compound(command, redirects) => {
            walk_compound(command, source, visit);
            walk_redirects(redirects, source, visit);
        }
        Command::Function(FunctionDef { body, .. }) => walk_command(body, source, visit),
    }
}

fn walk_compound<F>(command: &CompoundCommand, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    match command {
        CompoundCommand::If(command) => {
            walk_commands(&command.condition, source, visit);
            walk_commands(&command.then_branch, source, visit);
            for (condition, body) in &command.elif_branches {
                walk_commands(condition, source, visit);
                walk_commands(body, source, visit);
            }
            if let Some(body) = &command.else_branch {
                walk_commands(body, source, visit);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                walk_words(words, source, visit);
            }
            walk_commands(&command.body, source, visit);
        }
        CompoundCommand::ArithmeticFor(command) => walk_commands(&command.body, source, visit),
        CompoundCommand::While(command) => {
            walk_commands(&command.condition, source, visit);
            walk_commands(&command.body, source, visit);
        }
        CompoundCommand::Until(command) => {
            walk_commands(&command.condition, source, visit);
            walk_commands(&command.body, source, visit);
        }
        CompoundCommand::Case(command) => {
            walk_word(&command.word, source, visit);
            for case in &command.cases {
                walk_patterns(&case.patterns, source, visit);
                walk_commands(&case.commands, source, visit);
            }
        }
        CompoundCommand::Select(command) => {
            walk_words(&command.words, source, visit);
            walk_commands(&command.body, source, visit);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            walk_commands(commands, source, visit);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                walk_command(command, source, visit);
            }
        }
        CompoundCommand::Conditional(command) => {
            walk_conditional_expr(&command.expression, source, visit)
        }
        CompoundCommand::Coproc(command) => walk_command(&command.body, source, visit),
    }
}

fn walk_assignments<F>(assignments: &[Assignment], source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for assignment in assignments {
        walk_assignment(assignment, source, visit);
    }
}

fn walk_assignment<F>(assignment: &Assignment, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    walk_var_ref_subscript(&assignment.target, source, visit);
    match &assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, source, visit),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => walk_word(word, source, visit),
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        walk_subscript(Some(key), source, visit);
                        walk_word(value, source, visit)
                    }
                }
            }
        }
    }
}

fn walk_words<F>(words: &[Word], source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for word in words {
        walk_word(word, source, visit);
    }
}

fn walk_patterns<F>(patterns: &[Pattern], source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for pattern in patterns {
        walk_pattern(pattern, source, visit);
    }
}

fn walk_word<F>(word: &Word, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    walk_word_parts(&word.parts, source, visit);
}

fn walk_pattern<F>(pattern: &Pattern, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => walk_patterns(patterns, source, visit),
            PatternPart::Word(word) => walk_word(word, source, visit),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn walk_word_parts<F>(parts: &[WordPartNode], source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => walk_word_parts(parts, source, visit),
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    query::visit_arithmetic_words(expression_ast, &mut |word| {
                        walk_word(word, source, visit);
                    });
                }
            }
            WordPart::CommandSubstitution { commands, .. }
            | WordPart::ProcessSubstitution { commands, .. } => {
                walk_commands(commands, source, visit)
            }
            _ => {}
        }
    }
}

fn walk_redirects<F>(redirects: &[Redirect], source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    for redirect in redirects {
        let word = match redirect.word_target() {
            Some(word) => word,
            None => &redirect.heredoc().expect("expected heredoc redirect").body,
        };
        walk_word(word, source, visit);
    }
}

fn walk_conditional_expr<F>(expression: &ConditionalExpr, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    match expression {
        ConditionalExpr::Binary(expr) => {
            walk_conditional_expr(&expr.left, source, visit);
            walk_conditional_expr(&expr.right, source, visit);
        }
        ConditionalExpr::Unary(expr) => walk_conditional_expr(&expr.expr, source, visit),
        ConditionalExpr::Parenthesized(expr) => walk_conditional_expr(&expr.expr, source, visit),
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            walk_word(word, source, visit)
        }
        ConditionalExpr::Pattern(pattern) => walk_pattern(pattern, source, visit),
        ConditionalExpr::VarRef(reference) => {
            query::visit_var_ref_subscript_words(reference, source, &mut |word| {
                walk_word(word, source, visit);
            });
        }
    }
}

fn walk_var_ref_subscript<F>(reference: &shuck_ast::VarRef, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    walk_subscript(reference.subscript.as_ref(), source, visit);
}

fn walk_subscript<F>(subscript: Option<&Subscript>, source: &str, visit: &mut F)
where
    F: FnMut(Span),
{
    let Some(subscript) = subscript else {
        return;
    };
    if subscript.selector().is_some() {
        return;
    }
    if let Some(expression_ast) = subscript.arithmetic_ast.as_ref() {
        query::visit_arithmetic_words(expression_ast, &mut |word| {
            walk_word(word, source, visit);
        });
        return;
    }

    let text = subscript.syntax_source_text();
    let word = Parser::parse_word_fragment(source, text.slice(source), text.span());
    walk_word(&word, source, visit);
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(BuiltinCommand::Break(command)) => command.span,
        Command::Builtin(BuiltinCommand::Continue(command)) => command.span,
        Command::Builtin(BuiltinCommand::Return(command)) => command.span,
        Command::Builtin(BuiltinCommand::Exit(command)) => command.span,
        Command::Decl(command) => command.span,
        Command::Pipeline(command) => command.span,
        Command::List(CommandList { span, .. }) => *span,
        Command::Compound(command, _) => command_span_from_compound(command),
        Command::Function(command) => command.span,
    }
}

fn command_span_from_compound(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .first()
            .map(command_span)
            .zip(commands.last().map(command_span))
            .map(|(start, end)| start.merge(end))
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
    }
}

#[cfg(test)]
mod tests {
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;

    use super::*;
    use crate::{ShellCheckCodeMap, parse_directives};

    fn suppression_index(source: &str) -> SuppressionIndex {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        SuppressionIndex::new(
            &directives,
            &output.script,
            source,
            first_statement_line(&output.script).unwrap_or(u32::MAX),
        )
    }

    #[test]
    fn applies_disable_file_directives_to_the_entire_file() {
        let source = "echo $foo # shuck: disable-file=C006\n";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UndefinedVariable, 1));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 200));
    }

    #[test]
    fn applies_region_disable_until_a_matching_enable() {
        let source = "\
echo $foo
# shuck: disable=C006
echo $foo
# shuck: enable=C006
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UndefinedVariable, 1));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 2));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 3));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 4));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 5));
    }

    #[test]
    fn applies_dead_code_alias_suppressions() {
        let source = "\
exit 0
# shuck: disable=SH-293
echo dead
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 1));
        assert!(index.is_suppressed(Rule::UnreachableAfterExit, 2));
        assert!(index.is_suppressed(Rule::UnreachableAfterExit, 3));
    }

    #[test]
    fn promotes_shellcheck_directives_before_the_first_statement_to_file_scope() {
        let source = "\
#!/bin/bash
# shellcheck disable=SC2086

echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 1));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
    }

    #[test]
    fn scopes_shellcheck_disable_to_the_next_multiline_command() {
        let source = "\
echo ready
# shellcheck disable=SC2086
if true; then
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 6));
    }

    #[test]
    fn finds_next_commands_inside_command_substitutions() {
        let source = "\
echo ready
value=\"$(
  # shellcheck disable=SC2086
  echo $foo
)\"
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 6));
    }

    #[test]
    fn scopes_shellcheck_disable_to_keyed_array_subscript_diagnostics() {
        let source = "\
echo ready
# shellcheck disable=SC2016
declare -A map=(['$HOME']=1)
declare -A other=(['$HOME']=1)
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::SingleQuotedLiteral, 3));
        assert!(!index.is_suppressed(Rule::SingleQuotedLiteral, 4));
    }
}
