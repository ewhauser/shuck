use rustc_hash::FxHashMap;
use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BuiltinCommand, Command, CompoundCommand, ConditionalExpr, DeclOperand, File, FunctionDef,
    HeredocBodyPartNode, Pattern, PatternPart, Redirect, Span, Stmt, StmtSeq, TextSize, VarRef,
    Word, WordPart, WordPartNode,
};

use crate::Rule;

use super::{SuppressionAction, SuppressionDirective};

/// Per-file suppression index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuppressionIndex {
    by_rule: FxHashMap<Rule, RuleSuppressionIndex>,
}

impl SuppressionIndex {
    /// Build from parsed directives.
    pub fn new(directives: &[SuppressionDirective], file: &File, first_stmt_line: u32) -> Self {
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
                    SuppressionAction::Ignore => index.lines.push(directive.line),
                    SuppressionAction::Disable => {
                        if directive.line < first_stmt_line {
                            index.whole_file = true;
                        } else if let Some(range) = next_command_range(file, directive.range.end())
                        {
                            index.ranges.push(range);
                        }
                    }
                }
            }
        }

        for index in by_rule.values_mut() {
            index.lines.sort_unstable();
            index.lines.dedup();
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
pub fn first_statement_line(file: &File) -> Option<u32> {
    file.body
        .iter()
        .filter_map(|command| u32::try_from(command.span.start.line).ok())
        .min()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuleSuppressionIndex {
    whole_file: bool,
    lines: Vec<u32>,
    ranges: Vec<LineRange>,
}

impl RuleSuppressionIndex {
    fn is_suppressed(&self, line: u32) -> bool {
        if self.whole_file {
            return true;
        }

        if self.lines.binary_search(&line).is_ok() {
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
        false
    }
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

fn next_command_range(file: &File, offset: TextSize) -> Option<LineRange> {
    let mut next = None;
    for command in file.body.iter() {
        walk_command(command, &mut |span| {
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

fn walk_commands<F>(commands: &StmtSeq, visit: &mut F)
where
    F: FnMut(Span),
{
    for command in commands.iter() {
        walk_command(command, visit);
    }
}

fn walk_command<F>(stmt: &Stmt, visit: &mut F)
where
    F: FnMut(Span),
{
    visit(stmt.span);

    match &stmt.command {
        Command::Simple(command) => {
            walk_assignments(&command.assignments, visit);
            walk_word(&command.name, visit);
            walk_words(&command.args, visit);
        }
        Command::Builtin(BuiltinCommand::Break(command)) => {
            walk_assignments(&command.assignments, visit);
            if let Some(word) = &command.depth {
                walk_word(word, visit);
            }
            walk_words(&command.extra_args, visit);
        }
        Command::Builtin(BuiltinCommand::Continue(command)) => {
            walk_assignments(&command.assignments, visit);
            if let Some(word) = &command.depth {
                walk_word(word, visit);
            }
            walk_words(&command.extra_args, visit);
        }
        Command::Builtin(BuiltinCommand::Return(command)) => {
            walk_assignments(&command.assignments, visit);
            if let Some(word) = &command.code {
                walk_word(word, visit);
            }
            walk_words(&command.extra_args, visit);
        }
        Command::Builtin(BuiltinCommand::Exit(command)) => {
            walk_assignments(&command.assignments, visit);
            if let Some(word) = &command.code {
                walk_word(word, visit);
            }
            walk_words(&command.extra_args, visit);
        }
        Command::Decl(command) => {
            walk_assignments(&command.assignments, visit);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => walk_word(word, visit),
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => walk_assignment(assignment, visit),
                }
            }
        }
        Command::Binary(command) => {
            walk_command(&command.left, visit);
            walk_command(&command.right, visit);
        }
        Command::Compound(command) => {
            walk_compound(command, visit);
        }
        Command::Function(FunctionDef { header, body, .. }) => {
            for entry in &header.entries {
                walk_word(&entry.word, visit);
            }
            walk_command(body, visit);
        }
        Command::AnonymousFunction(function) => {
            walk_words(&function.args, visit);
            walk_command(&function.body, visit);
        }
    }

    walk_redirects(&stmt.redirects, visit);
}

fn walk_compound<F>(command: &CompoundCommand, visit: &mut F)
where
    F: FnMut(Span),
{
    match command {
        CompoundCommand::If(command) => {
            walk_commands(&command.condition, visit);
            walk_commands(&command.then_branch, visit);
            for (condition, body) in &command.elif_branches {
                walk_commands(condition, visit);
                walk_commands(body, visit);
            }
            if let Some(body) = &command.else_branch {
                walk_commands(body, visit);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                walk_words(words, visit);
            }
            walk_commands(&command.body, visit);
        }
        CompoundCommand::Repeat(command) => {
            walk_word(&command.count, visit);
            walk_commands(&command.body, visit);
        }
        CompoundCommand::Foreach(command) => {
            walk_words(&command.words, visit);
            walk_commands(&command.body, visit);
        }
        CompoundCommand::ArithmeticFor(command) => walk_commands(&command.body, visit),
        CompoundCommand::While(command) => {
            walk_commands(&command.condition, visit);
            walk_commands(&command.body, visit);
        }
        CompoundCommand::Until(command) => {
            walk_commands(&command.condition, visit);
            walk_commands(&command.body, visit);
        }
        CompoundCommand::Case(command) => {
            walk_word(&command.word, visit);
            for case in &command.cases {
                walk_patterns(&case.patterns, visit);
                walk_commands(&case.body, visit);
            }
        }
        CompoundCommand::Select(command) => {
            walk_words(&command.words, visit);
            walk_commands(&command.body, visit);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            walk_commands(commands, visit);
        }
        CompoundCommand::Always(command) => {
            walk_commands(&command.body, visit);
            walk_commands(&command.always_body, visit);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                walk_command(command, visit);
            }
        }
        CompoundCommand::Conditional(command) => walk_conditional_expr(&command.expression, visit),
        CompoundCommand::Coproc(command) => walk_command(&command.body, visit),
    }
}

fn walk_assignments<F>(assignments: &[Assignment], visit: &mut F)
where
    F: FnMut(Span),
{
    for assignment in assignments {
        walk_assignment(assignment, visit);
    }
}

fn walk_assignment<F>(assignment: &Assignment, visit: &mut F)
where
    F: FnMut(Span),
{
    match &assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, visit),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => walk_word(word, visit),
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        walk_word(value, visit)
                    }
                }
            }
        }
    }
}

fn walk_words<F>(words: &[Word], visit: &mut F)
where
    F: FnMut(Span),
{
    for word in words {
        walk_word(word, visit);
    }
}

fn walk_patterns<F>(patterns: &[Pattern], visit: &mut F)
where
    F: FnMut(Span),
{
    for pattern in patterns {
        walk_pattern(pattern, visit);
    }
}

fn walk_word<F>(word: &Word, visit: &mut F)
where
    F: FnMut(Span),
{
    walk_word_parts(&word.parts, visit);
}

fn walk_pattern<F>(pattern: &Pattern, visit: &mut F)
where
    F: FnMut(Span),
{
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => walk_patterns(patterns, visit),
            PatternPart::Word(word) => walk_word(word, visit),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn walk_word_parts<F>(parts: &[WordPartNode], visit: &mut F)
where
    F: FnMut(Span),
{
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => walk_word_parts(parts, visit),
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    visit_arithmetic_words(expression_ast, &mut |word| {
                        walk_word(word, visit);
                    });
                }
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => walk_commands(body, visit),
            _ => {}
        }
    }
}

fn walk_redirects<F>(redirects: &[Redirect], visit: &mut F)
where
    F: FnMut(Span),
{
    for redirect in redirects {
        if let Some(word) = redirect.word_target() {
            walk_word(word, visit);
        } else if let Some(heredoc) = redirect.heredoc()
            && heredoc.delimiter.expands_body
        {
            walk_heredoc_body_parts(&heredoc.body.parts, visit);
        }
    }
}

fn walk_heredoc_body_parts<F>(parts: &[HeredocBodyPartNode], visit: &mut F)
where
    F: FnMut(Span),
{
    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    visit_arithmetic_words(expression_ast, &mut |word| {
                        walk_word(word, visit);
                    });
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                walk_commands(body, visit)
            }
            shuck_ast::HeredocBodyPart::Literal(_)
            | shuck_ast::HeredocBodyPart::Variable(_)
            | shuck_ast::HeredocBodyPart::Parameter(_) => {}
        }
    }
}

fn walk_conditional_expr<F>(expression: &ConditionalExpr, visit: &mut F)
where
    F: FnMut(Span),
{
    match expression {
        ConditionalExpr::Binary(expr) => {
            walk_conditional_expr(&expr.left, visit);
            walk_conditional_expr(&expr.right, visit);
        }
        ConditionalExpr::Unary(expr) => walk_conditional_expr(&expr.expr, visit),
        ConditionalExpr::Parenthesized(expr) => walk_conditional_expr(&expr.expr, visit),
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => walk_word(word, visit),
        ConditionalExpr::Pattern(pattern) => walk_pattern(pattern, visit),
        ConditionalExpr::VarRef(reference) => {
            visit_var_ref_subscript_words(reference, &mut |word| {
                walk_word(word, visit);
            });
        }
    }
}

fn visit_var_ref_subscript_words<'a>(reference: &'a VarRef, visitor: &mut impl FnMut(&'a Word)) {
    if let Some(expression) = reference
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_deref())
    {
        visit_arithmetic_words(expression, visitor);
    }
}

fn visit_arithmetic_words<'a>(
    expression: &'a ArithmeticExprNode,
    visitor: &mut impl FnMut(&'a Word),
) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => visit_arithmetic_words(index, visitor),
        ArithmeticExpr::ShellWord(word) => visitor(word),
        ArithmeticExpr::Parenthesized { expression } => {
            visit_arithmetic_words(expression, visitor);
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visit_arithmetic_words(expr, visitor);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visit_arithmetic_words(left, visitor);
            visit_arithmetic_words(right, visitor);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arithmetic_words(condition, visitor);
            visit_arithmetic_words(then_expr, visitor);
            visit_arithmetic_words(else_expr, visitor);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue_words(target, visitor);
            visit_arithmetic_words(value, visitor);
        }
    }
}

fn visit_arithmetic_lvalue_words<'a>(
    target: &'a ArithmeticLvalue,
    visitor: &mut impl FnMut(&'a Word),
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => visit_arithmetic_words(index, visitor),
    }
}

#[cfg(test)]
mod tests {
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect};

    use super::*;
    use crate::{ShellCheckCodeMap, parse_directives};

    fn suppression_index(source: &str) -> SuppressionIndex {
        suppression_index_with_dialect(source, ShellDialect::Bash)
    }

    fn suppression_index_with_dialect(source: &str, dialect: ShellDialect) -> SuppressionIndex {
        let output = Parser::with_dialect(source, dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            &output.file,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        SuppressionIndex::new(
            &directives,
            &output.file,
            first_statement_line(&output.file).unwrap_or(u32::MAX),
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
    fn applies_shuck_disable_to_the_next_command() {
        let source = "\
foo='a b'
echo $foo
# shuck: disable=C006
echo $foo
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UndefinedVariable, 2));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 4));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 5));
    }

    #[test]
    fn applies_shuck_ignore_only_to_the_directive_line() {
        let source = "\
foo='a b'
echo $foo # shuck: ignore=C006
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UndefinedVariable, 1));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 2));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 3));
    }

    #[test]
    fn applies_dead_code_alias_suppressions() {
        let source = "\
exit 0
# shuck: disable=SH-293
echo dead
echo still_dead
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 1));
        assert!(index.is_suppressed(Rule::UnreachableAfterExit, 3));
        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 4));
    }

    #[test]
    fn applies_legacy_shellcheck_dead_code_suppression_alias() {
        let source = "\
exit 0
# shellcheck disable=SC2365
echo dead
echo still_dead
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 1));
        assert!(index.is_suppressed(Rule::UnreachableAfterExit, 3));
        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 4));
    }

    #[test]
    fn applies_shuck_disable_with_shellcheck_codes_to_the_next_command() {
        let source = "\
foo='a b'
echo $foo
# shuck: disable=SC2086
echo $foo
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 2));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn promotes_shuck_disable_before_the_first_statement_to_file_scope() {
        let source = "\
#!/bin/bash
# shuck: disable=S001

echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 1));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
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
    fn promotes_shellcheck_disable_all_before_the_first_statement_to_file_scope() {
        let source = "\
#!/bin/bash
# shellcheck disable=all

[ \"$a\" = 1 -a \"$b\" = 2 ]
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::CompoundTestOperator, 4));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_with_shuck_codes_to_the_next_command() {
        let source = "\
foo='a b'
# shellcheck disable=S001
echo $foo
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 4));
    }

    #[test]
    fn scopes_shuck_disable_after_then_header_to_the_next_command() {
        let source = "\
foo='a b'
if true; then # shuck: disable=S001
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
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
    fn scopes_shellcheck_disable_to_the_next_function_body() {
        let source = "\
# shellcheck disable=SC2059
plugin_current_command() {
  printf \"$terminal_format\" \"$plugin\" \"$version\" \"$description\" 1>&2
}
printf \"$later\" value
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::PrintfFormatVariable, 3));
        assert!(index.is_suppressed(Rule::PrintfFormatVariable, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_case_label_to_the_next_command() {
        let source = "\
case $x in
  on) # shellcheck disable=SC2086
    echo $foo
    ;;
esac
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_if_header_to_the_next_command() {
        let source = "\
foo='a b'
if # shellcheck disable=SC2086
  echo $foo
then
  echo $foo
fi
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_then_header_to_the_next_command() {
        let source = "\
foo='a b'
if true; then # shellcheck disable=SC2086
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_elif_then_header_to_the_next_command() {
        let source = "\
foo='a b'
if false; then
  :
elif true; then # shellcheck disable=SC2086
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 7));
    }

    #[test]
    fn scopes_shellcheck_disable_after_for_do_header_to_the_next_command() {
        let source = "\
foo='a b'
for item in 1; do # shellcheck disable=SC2086
  echo $foo
done
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_brace_group_opener_to_the_next_command() {
        let source = "\
foo='a b'
{ # shellcheck disable=SC2086
  echo $foo
}
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_then_inline_brace_group_opener() {
        let source = "\
foo='a b'
if true; then { # shellcheck disable=SC2086
  echo $foo
}; fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_then_inline_subshell_opener() {
        let source = "\
foo='a b'
if true; then ( # shellcheck disable=SC2086
  echo $foo
); fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_zsh_brace_if_headers() {
        let source = "\
foo='a b'
if [[ -n $foo ]] { # shellcheck disable=SC2086
  echo $foo
} elif [[ -n $foo ]] { # shellcheck disable=SC2086
  echo $foo
} else { # shellcheck disable=SC2086
  echo $foo
}
echo $foo
";
        let index = suppression_index_with_dialect(source, ShellDialect::Zsh);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 7));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 9));
    }

    #[test]
    fn scopes_shellcheck_disable_after_zsh_brace_loop_headers() {
        let source = "\
foo='a b'
for item in 1; { # shellcheck disable=SC2086
  echo $foo
}
repeat 2 { # shellcheck disable=SC2086
  echo $foo
}
foreach item (1 2) { # shellcheck disable=SC2086
  echo $foo
}
echo $foo
";
        let index = suppression_index_with_dialect(source, ShellDialect::Zsh);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 6));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 9));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 11));
    }

    #[test]
    fn ignores_shellcheck_directives_after_keyword_like_arguments() {
        let source = "\
echo if # shellcheck disable=SC2086
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 2));
    }

    #[test]
    fn ignores_shellcheck_directives_after_keyword_suffixes_inside_words() {
        let source = "\
foo='a b'
for item in to-do # shellcheck disable=SC2086
do
  echo $foo
done
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 6));
    }

    #[test]
    fn ignores_case_label_directives_after_same_line_body_commands() {
        let source = "\
case $x in
  on) echo $foo # shellcheck disable=SC2086
    echo $bar
    ;;
esac
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 2));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 3));
    }

    #[test]
    fn scopes_case_label_directives_inside_command_substitution_arguments() {
        let source = "\
printf '%s\\n' \"$(
  case $x in
    on) # shellcheck disable=SC2086
      echo $foo
      ;;
  esac
  echo $bar
)\"\n";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 7));
    }
}
