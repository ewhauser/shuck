use std::collections::BTreeMap;

use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, CaseItem, Command, CompoundCommand,
    ConditionalExpr, Script, Span, Word, WordPart,
};

use crate::{
    Directive, DirectiveSource, ParsedSyntax, SuppressionAction,
    SuppressionDirective as ParsedSuppressionDirective,
};

/// Suppression scope represented by the current index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuppressionKind {
    File,
    Region,
}

/// Effective suppression state for a rule code at a given line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    pub kind: SuppressionKind,
    pub source: DirectiveSource,
    pub codes: Vec<String>,
    pub line: usize,
    pub comment_text: String,
}

#[derive(Debug, Clone)]
struct SuppressionStateEvent {
    line: usize,
    file_directive: Option<Suppression>,
    active_directive: Option<Suppression>,
}

#[derive(Debug, Clone)]
struct SuppressionLineRange {
    start_line: usize,
    end_line: usize,
    suppression: Suppression,
}

#[derive(Debug, Clone)]
struct CodeSuppressionIndex {
    events: Vec<SuppressionStateEvent>,
    ranges: Vec<SuppressionLineRange>,
    whole_file: Option<Suppression>,
}

/// Line-based suppression lookup built from parsed directives and command spans.
#[derive(Debug, Clone, Default)]
pub struct SuppressionIndex {
    by_code: BTreeMap<String, CodeSuppressionIndex>,
}

impl SuppressionIndex {
    pub fn new(parsed: &ParsedSyntax) -> Self {
        Self::from_parts(&parsed.script, &parsed.directives)
    }

    pub(crate) fn from_parts(script: &Script, directives: &[Directive]) -> Self {
        let mut directives_by_code: BTreeMap<String, Vec<ParsedSuppressionDirective>> =
            BTreeMap::new();
        for directive in directives {
            let Directive::Suppression(directive) = directive else {
                continue;
            };
            for code in &directive.codes {
                directives_by_code
                    .entry(code.clone())
                    .or_default()
                    .push(directive.clone());
            }
        }

        let command_ranges = collect_command_ranges(script);
        let first_command_line = script
            .commands
            .iter()
            .filter_map(command_line_range)
            .map(|range| range.start_line)
            .min();

        let by_code = directives_by_code
            .into_iter()
            .map(|(code, directives)| {
                let index = build_code_index(&directives, &command_ranges, first_command_line);
                (code, index)
            })
            .collect();

        Self { by_code }
    }

    pub fn match_line(&self, code: &str, line: usize) -> Option<Suppression> {
        let code = normalize_query_code(code);
        self.by_code
            .get(&code)
            .and_then(|index| index.match_line(line))
    }

    pub fn match_line_with_aliases<'a, I>(&self, line: usize, codes: I) -> Option<Suppression>
    where
        I: IntoIterator<Item = &'a str>,
    {
        for code in codes {
            if let Some(suppression) = self.match_line(code, line) {
                return Some(suppression);
            }
        }
        None
    }

    pub fn suppresses_whole_file(&self, code: &str) -> bool {
        let code = normalize_query_code(code);
        self.by_code
            .get(&code)
            .and_then(|index| index.whole_file.as_ref())
            .is_some()
    }

    pub fn suppresses_whole_file_with_aliases<'a, I>(&self, codes: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        codes
            .into_iter()
            .any(|code| self.suppresses_whole_file(code))
    }
}

impl CodeSuppressionIndex {
    fn match_line(&self, line: usize) -> Option<Suppression> {
        if let Some(whole_file) = &self.whole_file {
            return Some(whole_file.clone());
        }

        let state = state_at_line(&self.events, line);
        if let Some(state) = &state
            && let Some(file_directive) = &state.file_directive
        {
            return Some(file_directive.clone());
        }

        for range in &self.ranges {
            if line >= range.start_line && line <= range.end_line {
                return Some(range.suppression.clone());
            }
        }

        state.and_then(|event| event.active_directive.clone())
    }
}

#[derive(Debug, Clone, Copy)]
struct CommandLineRange {
    start_line: usize,
    end_line: usize,
    start_offset: usize,
    end_offset: usize,
}

fn build_code_index(
    directives: &[ParsedSuppressionDirective],
    command_ranges: &[CommandLineRange],
    first_command_line: Option<usize>,
) -> CodeSuppressionIndex {
    let mut directives = directives.to_vec();
    directives.sort_by(|left, right| {
        left.span
            .start
            .offset
            .cmp(&right.span.start.offset)
            .then(left.span.end.offset.cmp(&right.span.end.offset))
    });

    let mut events = Vec::new();
    let mut ranges = Vec::new();
    let mut whole_file = None;
    let mut file_directive = None;
    let mut active_directive = None;

    for directive in directives {
        let suppression = suppression_from_directive(&directive);

        if directive.source == DirectiveSource::ShellCheck
            && directive.action == SuppressionAction::Disable
        {
            if first_command_line.is_none_or(|line| directive.span.start.line < line) {
                whole_file = Some(suppression);
                continue;
            }

            if let Some(range) = next_command_line_range(command_ranges, directive.span.end.offset)
            {
                ranges.push(SuppressionLineRange {
                    start_line: range.start_line,
                    end_line: range.end_line,
                    suppression,
                });
            }
            continue;
        }

        match directive.action {
            SuppressionAction::DisableFile => file_directive = Some(suppression),
            SuppressionAction::Disable => active_directive = Some(suppression),
            SuppressionAction::Enable => active_directive = None,
        }

        events.push(SuppressionStateEvent {
            line: directive.span.start.line,
            file_directive: file_directive.clone(),
            active_directive: active_directive.clone(),
        });
    }

    ranges.sort_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then(left.end_line.cmp(&right.end_line))
            .then(left.suppression.line.cmp(&right.suppression.line))
    });

    if whole_file.is_none()
        && let Some(first_command_line) = first_command_line
    {
        whole_file = whole_file_suppression(&events, first_command_line);
    }

    CodeSuppressionIndex {
        events,
        ranges,
        whole_file,
    }
}

fn suppression_from_directive(directive: &ParsedSuppressionDirective) -> Suppression {
    let kind = if directive.action == SuppressionAction::DisableFile {
        SuppressionKind::File
    } else {
        SuppressionKind::Region
    };

    Suppression {
        kind,
        source: directive.source,
        codes: directive.codes.clone(),
        line: directive.span.start.line,
        comment_text: directive.comment_text.clone(),
    }
}

fn state_at_line(events: &[SuppressionStateEvent], line: usize) -> Option<&SuppressionStateEvent> {
    let index = events.partition_point(|event| event.line <= line);
    index.checked_sub(1).map(|index| &events[index])
}

fn whole_file_suppression(
    events: &[SuppressionStateEvent],
    first_command_line: usize,
) -> Option<Suppression> {
    let start_state = state_at_line(events, first_command_line)?;
    if let Some(file_directive) = &start_state.file_directive {
        return Some(file_directive.clone());
    }

    let active_directive = start_state.active_directive.clone()?;
    let start_index = events.partition_point(|event| event.line <= first_command_line);
    if events[start_index..]
        .iter()
        .any(|event| event.file_directive.is_none() && event.active_directive.is_none())
    {
        return None;
    }

    Some(active_directive)
}

fn next_command_line_range(
    command_ranges: &[CommandLineRange],
    after_offset: usize,
) -> Option<CommandLineRange> {
    command_ranges
        .iter()
        .copied()
        .find(|range| range.start_offset > after_offset)
}

fn collect_command_ranges(script: &Script) -> Vec<CommandLineRange> {
    let mut ranges = Vec::new();
    for command in &script.commands {
        collect_command_ranges_from_command(command, &mut ranges);
    }

    ranges.sort_by(|left, right| {
        left.start_offset
            .cmp(&right.start_offset)
            .then(right.end_offset.cmp(&left.end_offset))
    });
    ranges.dedup_by(|left, right| {
        left.start_offset == right.start_offset
            && left.end_offset == right.end_offset
            && left.start_line == right.start_line
            && left.end_line == right.end_line
    });
    ranges
}

fn collect_command_ranges_from_command(command: &Command, ranges: &mut Vec<CommandLineRange>) {
    if let Some(range) = command_line_range(command) {
        ranges.push(range);
    }

    match command {
        Command::Simple(command) => collect_command_ranges_from_simple(command, ranges),
        Command::Builtin(command) => collect_command_ranges_from_builtin(command, ranges),
        Command::Pipeline(pipeline) => {
            for command in &pipeline.commands {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        Command::List(list) => {
            collect_command_ranges_from_command(&list.first, ranges);
            for (_, command) in &list.rest {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        Command::Compound(compound, redirects) => {
            collect_command_ranges_from_compound(compound, ranges);
            for redirect in redirects {
                collect_command_ranges_from_word(&redirect.target, ranges);
            }
        }
        Command::Function(function) => {
            collect_command_ranges_from_command(&function.body, ranges);
        }
    }
}

fn collect_command_ranges_from_builtin(
    command: &BuiltinCommand,
    ranges: &mut Vec<CommandLineRange>,
) {
    match command {
        BuiltinCommand::Break(command) => {
            if let Some(depth) = &command.depth {
                collect_command_ranges_from_word(depth, ranges);
            }
            for word in &command.extra_args {
                collect_command_ranges_from_word(word, ranges);
            }
            for redirect in &command.redirects {
                collect_command_ranges_from_word(&redirect.target, ranges);
            }
            for assignment in &command.assignments {
                collect_command_ranges_from_assignment(assignment, ranges);
            }
        }
        BuiltinCommand::Continue(command) => {
            if let Some(depth) = &command.depth {
                collect_command_ranges_from_word(depth, ranges);
            }
            for word in &command.extra_args {
                collect_command_ranges_from_word(word, ranges);
            }
            for redirect in &command.redirects {
                collect_command_ranges_from_word(&redirect.target, ranges);
            }
            for assignment in &command.assignments {
                collect_command_ranges_from_assignment(assignment, ranges);
            }
        }
        BuiltinCommand::Return(command) => {
            if let Some(code) = &command.code {
                collect_command_ranges_from_word(code, ranges);
            }
            for word in &command.extra_args {
                collect_command_ranges_from_word(word, ranges);
            }
            for redirect in &command.redirects {
                collect_command_ranges_from_word(&redirect.target, ranges);
            }
            for assignment in &command.assignments {
                collect_command_ranges_from_assignment(assignment, ranges);
            }
        }
        BuiltinCommand::Exit(command) => {
            if let Some(code) = &command.code {
                collect_command_ranges_from_word(code, ranges);
            }
            for word in &command.extra_args {
                collect_command_ranges_from_word(word, ranges);
            }
            for redirect in &command.redirects {
                collect_command_ranges_from_word(&redirect.target, ranges);
            }
            for assignment in &command.assignments {
                collect_command_ranges_from_assignment(assignment, ranges);
            }
        }
    }
}

fn collect_command_ranges_from_simple(
    command: &shuck_ast::SimpleCommand,
    ranges: &mut Vec<CommandLineRange>,
) {
    collect_command_ranges_from_word(&command.name, ranges);
    for word in &command.args {
        collect_command_ranges_from_word(word, ranges);
    }
    for redirect in &command.redirects {
        collect_command_ranges_from_word(&redirect.target, ranges);
    }
    for assignment in &command.assignments {
        collect_command_ranges_from_assignment(assignment, ranges);
    }
}

fn collect_command_ranges_from_compound(
    compound: &CompoundCommand,
    ranges: &mut Vec<CommandLineRange>,
) {
    match compound {
        CompoundCommand::If(command) => {
            for condition in &command.condition {
                collect_command_ranges_from_command(condition, ranges);
            }
            for command in &command.then_branch {
                collect_command_ranges_from_command(command, ranges);
            }
            for (condition, branch) in &command.elif_branches {
                for command in condition {
                    collect_command_ranges_from_command(command, ranges);
                }
                for command in branch {
                    collect_command_ranges_from_command(command, ranges);
                }
            }
            if let Some(branch) = &command.else_branch {
                for command in branch {
                    collect_command_ranges_from_command(command, ranges);
                }
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                for word in words {
                    collect_command_ranges_from_word(word, ranges);
                }
            }
            for command in &command.body {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::ArithmeticFor(command) => {
            for command in &command.body {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::While(command) => {
            for condition in &command.condition {
                collect_command_ranges_from_command(condition, ranges);
            }
            for command in &command.body {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::Until(command) => {
            for condition in &command.condition {
                collect_command_ranges_from_command(condition, ranges);
            }
            for command in &command.body {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::Case(command) => {
            collect_command_ranges_from_word(&command.word, ranges);
            for item in &command.cases {
                collect_command_ranges_from_case_item(item, ranges);
            }
        }
        CompoundCommand::Select(command) => {
            for word in &command.words {
                collect_command_ranges_from_word(word, ranges);
            }
            for command in &command.body {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            for command in commands {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command_ranges_from_command(command, ranges);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_command_ranges_from_conditional_expr(&command.expression, ranges);
        }
        CompoundCommand::Coproc(command) => {
            collect_command_ranges_from_command(&command.body, ranges);
        }
    }
}

fn collect_command_ranges_from_case_item(item: &CaseItem, ranges: &mut Vec<CommandLineRange>) {
    for pattern in &item.patterns {
        collect_command_ranges_from_word(pattern, ranges);
    }
    for command in &item.commands {
        collect_command_ranges_from_command(command, ranges);
    }
}

fn collect_command_ranges_from_assignment(
    assignment: &Assignment,
    ranges: &mut Vec<CommandLineRange>,
) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_command_ranges_from_word(word, ranges),
        AssignmentValue::Array(words) => {
            for word in words {
                collect_command_ranges_from_word(word, ranges);
            }
        }
    }
}

fn collect_command_ranges_from_word(word: &Word, ranges: &mut Vec<CommandLineRange>) {
    for part in &word.parts {
        match part {
            WordPart::CommandSubstitution(commands)
            | WordPart::ProcessSubstitution { commands, .. } => {
                for command in commands {
                    collect_command_ranges_from_command(command, ranges);
                }
            }
            _ => {}
        }
    }
}

fn collect_command_ranges_from_conditional_expr(
    expr: &ConditionalExpr,
    ranges: &mut Vec<CommandLineRange>,
) {
    match expr {
        ConditionalExpr::Binary(expr) => {
            collect_command_ranges_from_conditional_expr(&expr.left, ranges);
            collect_command_ranges_from_conditional_expr(&expr.right, ranges);
        }
        ConditionalExpr::Unary(expr) => {
            collect_command_ranges_from_conditional_expr(&expr.expr, ranges);
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_command_ranges_from_conditional_expr(&expr.expr, ranges);
        }
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => collect_command_ranges_from_word(word, ranges),
    }
}

fn command_line_range(command: &Command) -> Option<CommandLineRange> {
    match command {
        Command::Simple(command) => span_line_range(command.span),
        Command::Builtin(command) => builtin_line_range(command),
        Command::Pipeline(command) => span_line_range(command.span),
        Command::List(command) => span_line_range(command.span),
        Command::Compound(compound, _) => compound_line_range(compound),
        Command::Function(command) => span_line_range(command.span),
    }
}

fn builtin_line_range(command: &BuiltinCommand) -> Option<CommandLineRange> {
    match command {
        BuiltinCommand::Break(command) => span_line_range(command.span),
        BuiltinCommand::Continue(command) => span_line_range(command.span),
        BuiltinCommand::Return(command) => span_line_range(command.span),
        BuiltinCommand::Exit(command) => span_line_range(command.span),
    }
}

fn compound_line_range(compound: &CompoundCommand) -> Option<CommandLineRange> {
    match compound {
        CompoundCommand::If(command) => span_line_range(command.span),
        CompoundCommand::For(command) => span_line_range(command.span),
        CompoundCommand::ArithmeticFor(command) => span_line_range(command.span),
        CompoundCommand::While(command) => span_line_range(command.span),
        CompoundCommand::Until(command) => span_line_range(command.span),
        CompoundCommand::Case(command) => span_line_range(command.span),
        CompoundCommand::Select(command) => span_line_range(command.span),
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            merge_command_line_ranges(commands)
        }
        CompoundCommand::Arithmetic(_) => None,
        CompoundCommand::Conditional(command) => span_line_range(command.span),
        CompoundCommand::Time(command) => span_line_range(command.span),
        CompoundCommand::Coproc(command) => span_line_range(command.span),
    }
}

fn merge_command_line_ranges(commands: &[Command]) -> Option<CommandLineRange> {
    let mut ranges = commands.iter().filter_map(command_line_range);
    let mut merged = ranges.next()?;
    for range in ranges {
        merged.start_line = merged.start_line.min(range.start_line);
        merged.end_line = merged.end_line.max(range.end_line);
        merged.start_offset = merged.start_offset.min(range.start_offset);
        merged.end_offset = merged.end_offset.max(range.end_offset);
    }
    Some(merged)
}

fn span_line_range(span: Span) -> Option<CommandLineRange> {
    if span.start.line == 0 || span.end.line == 0 {
        return None;
    }

    let end_line = if span.end.line > span.start.line && span.end.column == 1 {
        span.end.line - 1
    } else {
        span.end.line.max(span.start.line)
    };

    Some(CommandLineRange {
        start_line: span.start.line,
        end_line,
        start_offset: span.start.offset,
        end_offset: span.end.offset,
    })
}

fn normalize_query_code(code: &str) -> String {
    canonicalize_shuck_code(code)
        .or_else(|| canonicalize_shellcheck_code(code))
        .unwrap_or_else(|| code.trim().to_ascii_uppercase())
}

fn canonicalize_shuck_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();

    if let Some(digits) = code.strip_prefix("SH-") {
        return canonicalize_shuck_digits(digits);
    }
    if let Some(digits) = code.strip_prefix("SH") {
        return canonicalize_shuck_digits(digits);
    }

    None
}

fn canonicalize_shuck_digits(digits: &str) -> Option<String> {
    if digits.len() == 3 && digits.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("SH-{digits}"))
    } else {
        None
    }
}

fn canonicalize_shellcheck_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();

    if let Some(digits) = code.strip_prefix("SC")
        && !digits.is_empty()
        && digits.chars().all(|ch| ch.is_ascii_digit())
    {
        return Some(format!("SC{digits}"));
    }

    if !code.is_empty() && code.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(format!("SC{code}"));
    }

    None
}

#[cfg(test)]
mod tests {
    use crate::{ParseOptions, parse};

    use super::*;

    #[test]
    fn shuck_region_disable_stops_at_enable() {
        let parsed = parse(
            "#!/bin/bash\n# shuck: disable=SH001\necho $x\n# shuck: enable=SH001\necho $x\n",
            ParseOptions::default(),
        )
        .unwrap();

        let index = parsed.suppression_index();
        let suppression = index.match_line("SH001", 3).unwrap();
        assert_eq!(suppression.kind, SuppressionKind::Region);
        assert_eq!(suppression.source, DirectiveSource::Shuck);
        assert_eq!(suppression.line, 2);
        assert!(index.match_line("SH-001", 5).is_none());
    }

    #[test]
    fn inline_shellcheck_directives_do_not_suppress() {
        let parsed = parse(
            "#!/bin/sh\nx='a b'\necho $x # shellcheck disable=SC2086\necho $x\n",
            ParseOptions::default(),
        )
        .unwrap();

        let index = parsed.suppression_index();
        assert!(
            index
                .match_line_with_aliases(3, ["SH-001", "SC2086"])
                .is_none()
        );
        assert!(
            index
                .match_line_with_aliases(4, ["SH-001", "SC2086"])
                .is_none()
        );
    }

    #[test]
    fn shellcheck_disable_suppresses_next_compound_command_only() {
        let parsed = parse(
            "#!/bin/sh\nx='a b'\n# shellcheck disable=SC2086\n# note\ntest \"$x\" = ok || if true; then\n  echo $x\nfi\necho $x\n",
            ParseOptions::default(),
        )
        .unwrap();

        let index = parsed.suppression_index();
        let suppression = index
            .match_line_with_aliases(6, ["SH-001", "SC2086"])
            .unwrap();
        assert_eq!(suppression.kind, SuppressionKind::Region);
        assert_eq!(suppression.source, DirectiveSource::ShellCheck);
        assert_eq!(suppression.line, 3);
        assert!(
            index
                .match_line_with_aliases(8, ["SH-001", "SC2086"])
                .is_none()
        );
    }

    #[test]
    fn shellcheck_disable_applies_inside_command_substitution() {
        let parsed = parse(
            "#!/bin/sh\nx='a b'\nout=$(\n  # shellcheck disable=SC2086\n  printf '%s\\n' $x\n)\nprintf '%s\\n' $x\n",
            ParseOptions::default(),
        )
        .unwrap();

        let index = parsed.suppression_index();
        assert!(
            index
                .match_line_with_aliases(5, ["SH-001", "SC2086"])
                .is_some()
        );
        assert!(
            index
                .match_line_with_aliases(7, ["SH-001", "SC2086"])
                .is_none()
        );
    }

    #[test]
    fn shellcheck_header_disable_suppresses_whole_file() {
        let parsed = parse(
            "#!/bin/sh\n# note\n# shellcheck disable=SC2034\nfoo=bar\nbar=baz\n",
            ParseOptions::default(),
        )
        .unwrap();

        let index = parsed.suppression_index();
        assert!(index.suppresses_whole_file_with_aliases(["SH-003", "SC2034"]));
        assert!(
            index
                .match_line_with_aliases(4, ["SH-003", "SC2034"])
                .is_some()
        );
        assert!(
            index
                .match_line_with_aliases(5, ["SH-003", "SC2034"])
                .is_some()
        );
    }
}
