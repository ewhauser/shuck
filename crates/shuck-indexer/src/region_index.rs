use shuck_ast::{
    ArithmeticForCommand, Assignment, AssignmentValue, BuiltinCommand, Command, CommandList,
    CompoundCommand, ConditionalExpr, DeclClause, DeclOperand, FunctionDef, Redirect, RedirectKind,
    Script, TextRange, TextSize, Word, WordPart, WordPartNode,
};

/// A syntactic region that affects lint rule behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    SingleQuoted,
    DoubleQuoted,
    Heredoc,
    CommandSubstitution,
    Arithmetic,
    Conditional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexedRegion {
    kind: RegionKind,
    range: TextRange,
}

/// Byte ranges of syntactic regions where special rules apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionIndex {
    single_quoted: Vec<TextRange>,
    double_quoted: Vec<TextRange>,
    heredocs: Vec<TextRange>,
    command_substitutions: Vec<TextRange>,
    arithmetic: Vec<TextRange>,
    conditionals: Vec<TextRange>,
    quoted_heredocs: Vec<TextRange>,
    regions: Vec<IndexedRegion>,
}

impl RegionIndex {
    /// Build from source text and the parsed script.
    pub fn new(_source: &str, script: &Script) -> Self {
        let mut collector = RegionCollector::new();
        collector.visit_script(script);
        collector.finish()
    }

    /// Return the innermost region containing the given byte offset, if any.
    pub fn region_at(&self, offset: TextSize) -> Option<RegionKind> {
        self.region_with_range_at(offset).map(|(kind, _)| kind)
    }

    /// Return the innermost region kind and range containing the given byte offset, if any.
    pub fn region_with_range_at(&self, offset: TextSize) -> Option<(RegionKind, TextRange)> {
        let mut best: Option<IndexedRegion> = None;
        let end = self
            .regions
            .partition_point(|region| region.range.start() <= offset);

        for region in self.regions[..end].iter().copied() {
            if !contains(region.range, offset) {
                continue;
            }

            best = match best {
                None => Some(region),
                Some(current) if is_innermost(region.range, current.range) => Some(region),
                Some(current) => Some(current),
            };
        }

        best.map(|region| (region.kind, region.range))
    }

    /// Return the single-quoted range containing the given byte offset, if any.
    pub fn single_quoted_range_at(&self, offset: TextSize) -> Option<TextRange> {
        containing_range(&self.single_quoted, offset)
    }

    /// Return the double-quoted range containing the given byte offset, if any.
    pub fn double_quoted_range_at(&self, offset: TextSize) -> Option<TextRange> {
        containing_range(&self.double_quoted, offset)
    }

    /// Check if a byte offset falls inside any quoted region.
    pub fn is_quoted(&self, offset: TextSize) -> bool {
        contains_any(&self.single_quoted, offset)
            || contains_any(&self.double_quoted, offset)
            || contains_any(&self.quoted_heredocs, offset)
    }

    /// Check if a byte offset falls inside a heredoc body.
    pub fn is_heredoc(&self, offset: TextSize) -> bool {
        contains_any(&self.heredocs, offset)
    }

    /// Check if a byte offset falls inside a command substitution.
    pub fn is_command_substitution(&self, offset: TextSize) -> bool {
        contains_any(&self.command_substitutions, offset)
    }

    /// Check if a byte offset falls inside an arithmetic context.
    pub fn is_arithmetic(&self, offset: TextSize) -> bool {
        contains_any(&self.arithmetic, offset)
    }

    /// All heredoc body ranges.
    pub fn heredoc_ranges(&self) -> &[TextRange] {
        &self.heredocs
    }
}

struct RegionCollector {
    single_quoted: Vec<TextRange>,
    double_quoted: Vec<TextRange>,
    heredocs: Vec<TextRange>,
    command_substitutions: Vec<TextRange>,
    arithmetic: Vec<TextRange>,
    conditionals: Vec<TextRange>,
    quoted_heredocs: Vec<TextRange>,
}

impl RegionCollector {
    fn new() -> Self {
        Self {
            single_quoted: Vec::new(),
            double_quoted: Vec::new(),
            heredocs: Vec::new(),
            command_substitutions: Vec::new(),
            arithmetic: Vec::new(),
            conditionals: Vec::new(),
            quoted_heredocs: Vec::new(),
        }
    }

    fn finish(mut self) -> RegionIndex {
        sort_ranges(&mut self.single_quoted);
        sort_ranges(&mut self.double_quoted);
        sort_ranges(&mut self.heredocs);
        sort_ranges(&mut self.command_substitutions);
        sort_ranges(&mut self.arithmetic);
        sort_ranges(&mut self.conditionals);
        sort_ranges(&mut self.quoted_heredocs);

        let mut regions = Vec::with_capacity(
            self.single_quoted.len()
                + self.double_quoted.len()
                + self.heredocs.len()
                + self.command_substitutions.len()
                + self.arithmetic.len()
                + self.conditionals.len(),
        );
        regions.extend(
            self.single_quoted
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::SingleQuoted,
                    range,
                }),
        );
        regions.extend(
            self.double_quoted
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::DoubleQuoted,
                    range,
                }),
        );
        regions.extend(self.heredocs.iter().copied().map(|range| IndexedRegion {
            kind: RegionKind::Heredoc,
            range,
        }));
        regions.extend(
            self.command_substitutions
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::CommandSubstitution,
                    range,
                }),
        );
        regions.extend(self.arithmetic.iter().copied().map(|range| IndexedRegion {
            kind: RegionKind::Arithmetic,
            range,
        }));
        regions.extend(
            self.conditionals
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::Conditional,
                    range,
                }),
        );
        regions.sort_unstable_by_key(|region| {
            (region.range.start().to_u32(), region.range.end().to_u32())
        });

        RegionIndex {
            single_quoted: self.single_quoted,
            double_quoted: self.double_quoted,
            heredocs: self.heredocs,
            command_substitutions: self.command_substitutions,
            arithmetic: self.arithmetic,
            conditionals: self.conditionals,
            quoted_heredocs: self.quoted_heredocs,
            regions,
        }
    }

    fn visit_script(&mut self, script: &Script) {
        self.visit_commands(&script.commands);
    }

    fn visit_commands(&mut self, commands: &[Command]) {
        for command in commands {
            self.visit_command(command);
        }
    }

    fn visit_command(&mut self, command: &Command) {
        match command {
            Command::Simple(command) => {
                self.visit_word(&command.name, true);
                for argument in &command.args {
                    self.visit_word(argument, true);
                }
                for redirect in &command.redirects {
                    self.visit_redirect(redirect);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            Command::Builtin(command) => self.visit_builtin(command),
            Command::Decl(command) => self.visit_decl(command),
            Command::Pipeline(pipeline) => self.visit_commands(&pipeline.commands),
            Command::List(CommandList { first, rest, .. }) => {
                self.visit_command(first);
                for item in rest {
                    self.visit_command(&item.command);
                }
            }
            Command::Compound(command, redirects) => {
                self.visit_compound(command);
                for redirect in redirects {
                    self.visit_redirect(redirect);
                }
            }
            Command::Function(FunctionDef { body, .. }) => self.visit_command(body),
        }
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand) {
        match command {
            BuiltinCommand::Break(command) => {
                if let Some(depth) = &command.depth {
                    self.visit_word(depth, true);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, true);
                }
                for redirect in &command.redirects {
                    self.visit_redirect(redirect);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(depth) = &command.depth {
                    self.visit_word(depth, true);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, true);
                }
                for redirect in &command.redirects {
                    self.visit_redirect(redirect);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(code) = &command.code {
                    self.visit_word(code, true);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, true);
                }
                for redirect in &command.redirects {
                    self.visit_redirect(redirect);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(code) = &command.code {
                    self.visit_word(code, true);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, true);
                }
                for redirect in &command.redirects {
                    self.visit_redirect(redirect);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
        }
    }

    fn visit_decl(&mut self, command: &DeclClause) {
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.visit_word(word, true),
                DeclOperand::Name(_) => {}
                DeclOperand::Assignment(assignment) => self.visit_assignment(assignment),
            }
        }
        for redirect in &command.redirects {
            self.visit_redirect(redirect);
        }
        for assignment in &command.assignments {
            self.visit_assignment(assignment);
        }
    }

    fn visit_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
                self.visit_commands(&command.condition);
                self.visit_commands(&command.then_branch);
                for (condition, branch) in &command.elif_branches {
                    self.visit_commands(condition);
                    self.visit_commands(branch);
                }
                if let Some(branch) = &command.else_branch {
                    self.visit_commands(branch);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        self.visit_word(word, true);
                    }
                }
                self.visit_commands(&command.body);
            }
            CompoundCommand::ArithmeticFor(command) => {
                self.push_arithmetic_range(command);
                self.visit_commands(&command.body);
            }
            CompoundCommand::While(command) => {
                self.visit_commands(&command.condition);
                self.visit_commands(&command.body);
            }
            CompoundCommand::Until(command) => {
                self.visit_commands(&command.condition);
                self.visit_commands(&command.body);
            }
            CompoundCommand::Case(command) => {
                self.visit_word(&command.word, true);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        self.visit_word(pattern, true);
                    }
                    self.visit_commands(&item.commands);
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    self.visit_word(word, true);
                }
                self.visit_commands(&command.body);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.visit_commands(commands);
            }
            CompoundCommand::Arithmetic(command) => {
                push_range(&mut self.arithmetic, command.span.to_range());
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.visit_command(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                push_range(&mut self.conditionals, command.span.to_range());
                self.visit_conditional_expr(&command.expression);
            }
            CompoundCommand::Coproc(command) => self.visit_command(&command.body),
        }
    }

    fn push_arithmetic_range(&mut self, command: &ArithmeticForCommand) {
        let range = command
            .left_paren_span
            .merge(command.right_paren_span)
            .to_range();
        push_range(&mut self.arithmetic, range);
    }

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr) {
        match expression {
            ConditionalExpr::Binary(expression) => {
                self.visit_conditional_expr(&expression.left);
                self.visit_conditional_expr(&expression.right);
            }
            ConditionalExpr::Unary(expression) => self.visit_conditional_expr(&expression.expr),
            ConditionalExpr::Parenthesized(expression) => {
                self.visit_conditional_expr(&expression.expr);
            }
            ConditionalExpr::Word(word)
            | ConditionalExpr::Pattern(word)
            | ConditionalExpr::Regex(word) => self.visit_word(word, true),
        }
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        match redirect.kind {
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => {
                let heredoc = redirect.heredoc().expect("expected heredoc redirect");
                let range = heredoc.body.span.to_range();
                push_range(&mut self.heredocs, range);
                if is_fully_quoted_word(&heredoc.body) {
                    push_range(&mut self.quoted_heredocs, range);
                }
                self.visit_word_parts(&heredoc.body.parts);
            }
            _ => self.visit_word(
                redirect
                    .word_target()
                    .expect("expected non-heredoc redirect target"),
                true,
            ),
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.visit_word(word, true),
            AssignmentValue::Array(words) => {
                for word in words {
                    self.visit_word(word, true);
                }
            }
        }
    }

    fn visit_word(&mut self, word: &Word, scan_quotes: bool) {
        let _ = scan_quotes;
        self.visit_word_parts(&word.parts);
    }

    fn visit_word_parts(&mut self, parts: &[WordPartNode]) {
        for part in parts {
            let range = part.span.to_range();
            match &part.kind {
                WordPart::SingleQuoted { .. } => {
                    push_range(&mut self.single_quoted, range);
                }
                WordPart::DoubleQuoted { parts, .. } => {
                    push_range(&mut self.double_quoted, range);
                    self.visit_word_parts(parts);
                }
                WordPart::CommandSubstitution { commands, .. } => {
                    push_range(&mut self.command_substitutions, range);
                    self.visit_commands(commands);
                }
                WordPart::ArithmeticExpansion { .. } => {
                    push_range(&mut self.arithmetic, range);
                }
                WordPart::ProcessSubstitution { commands, .. } => self.visit_commands(commands),
                WordPart::Literal(_)
                | WordPart::Variable(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess { .. }
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch(_)
                | WordPart::Transformation { .. } => {}
            }
        }
    }
}

fn is_fully_quoted_word(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if matches!(
            &part.kind,
            WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
        )
    )
}

fn sort_ranges(ranges: &mut [TextRange]) {
    ranges.sort_unstable_by_key(|range| (range.start().to_u32(), range.end().to_u32()));
}

fn push_range(ranges: &mut Vec<TextRange>, range: TextRange) {
    if !range.is_empty() {
        ranges.push(range);
    }
}

fn contains(range: TextRange, offset: TextSize) -> bool {
    range.start() <= offset && offset < range.end()
}

fn contains_any(ranges: &[TextRange], offset: TextSize) -> bool {
    containing_range(ranges, offset).is_some()
}

fn containing_range(ranges: &[TextRange], offset: TextSize) -> Option<TextRange> {
    let index = ranges.partition_point(|range| range.start() <= offset);
    let mut best = None;

    for range in ranges[..index].iter().copied() {
        if !contains(range, offset) {
            continue;
        }

        best = match best {
            None => Some(range),
            Some(current) if is_innermost(range, current) => Some(range),
            Some(current) => Some(current),
        };
    }

    best
}

fn is_innermost(candidate: TextRange, current: TextRange) -> bool {
    candidate.len() < current.len()
        || (candidate.len() == current.len() && candidate.start() >= current.start())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn regions(source: &str) -> RegionIndex {
        let output = Parser::new(source).parse().unwrap();
        RegionIndex::new(source, &output.script)
    }

    #[test]
    fn finds_single_and_double_quoted_regions() {
        let source = "echo 'hello' \"world $name\"\n";
        let regions = regions(source);

        let single = TextSize::new(source.find("hello").unwrap() as u32);
        let double = TextSize::new(source.find("world").unwrap() as u32);

        assert_eq!(regions.region_at(single), Some(RegionKind::SingleQuoted));
        assert_eq!(regions.region_at(double), Some(RegionKind::DoubleQuoted));
        assert_eq!(
            regions
                .single_quoted_range_at(single)
                .unwrap()
                .slice(source),
            "'hello'"
        );
        assert_eq!(
            regions
                .double_quoted_range_at(double)
                .unwrap()
                .slice(source),
            "\"world $name\""
        );
    }

    #[test]
    fn finds_command_substitution_and_arithmetic_regions() {
        let source = "echo $(printf hi) $((1 + 2))\n";
        let regions = regions(source);

        let command = TextSize::new(source.find("printf").unwrap() as u32);
        let arithmetic = TextSize::new(source.find("1 + 2").unwrap() as u32);

        assert_eq!(
            regions.region_at(command),
            Some(RegionKind::CommandSubstitution)
        );
        assert_eq!(regions.region_at(arithmetic), Some(RegionKind::Arithmetic));
        assert!(regions.is_command_substitution(command));
        assert!(regions.is_arithmetic(arithmetic));
    }

    #[test]
    fn finds_heredoc_regions_and_tracks_quoted_heredocs() {
        let source = "cat <<'EOF'\nhello $name\nEOF\n";
        let regions = regions(source);
        let offset = TextSize::new(source.find("hello $name").unwrap() as u32);

        assert_eq!(regions.region_at(offset), Some(RegionKind::Heredoc));
        assert!(regions.is_heredoc(offset));
        assert!(regions.is_quoted(offset));
    }

    #[test]
    fn returns_the_innermost_nested_region() {
        let source = "echo \"$(printf '%s' \"$name\")\"\n";
        let regions = regions(source);

        let name = TextSize::new(source.find("$name").unwrap() as u32);
        let printf = TextSize::new(source.find("printf").unwrap() as u32);

        assert_eq!(regions.region_at(name), Some(RegionKind::DoubleQuoted));
        assert_eq!(
            regions.region_at(printf),
            Some(RegionKind::CommandSubstitution)
        );
        assert_eq!(
            regions.region_with_range_at(printf),
            Some((
                RegionKind::CommandSubstitution,
                TextRange::new(
                    TextSize::new(source.find("$(printf").unwrap() as u32),
                    TextSize::new(source.rfind(')').unwrap() as u32 + 1),
                )
            ))
        );
    }

    #[test]
    fn tracks_conditional_ranges() {
        let source = "[[ \"$name\" =~ foo ]]\n";
        let regions = regions(source);
        let offset = TextSize::new(source.find("foo").unwrap() as u32);

        assert_eq!(regions.region_at(offset), Some(RegionKind::Conditional));
    }

    #[test]
    fn quoted_range_helpers_return_none_outside_matching_quote_kind() {
        let source = "echo unquoted \"$name\"\n";
        let regions = regions(source);
        let unquoted = TextSize::new(source.find("unquoted").unwrap() as u32);
        let quoted = TextSize::new(source.find("$name").unwrap() as u32);

        assert_eq!(regions.single_quoted_range_at(unquoted), None);
        assert_eq!(regions.double_quoted_range_at(unquoted), None);
        assert_eq!(regions.single_quoted_range_at(quoted), None);
        assert_eq!(
            regions
                .double_quoted_range_at(quoted)
                .unwrap()
                .slice(source),
            "\"$name\""
        );
    }
}
