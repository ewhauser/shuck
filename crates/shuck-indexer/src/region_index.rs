use shuck_ast::{
    ArithmeticForCommand, Assignment, AssignmentValue, BuiltinCommand, Command, CommandList,
    CompoundCommand, ConditionalExpr, DeclClause, DeclOperand, FunctionDef, Redirect, RedirectKind,
    Script, Span, TextRange, TextSize, Word, WordPart,
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
    pub fn new(source: &str, script: &Script) -> Self {
        let mut collector = RegionCollector::new(source);
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

struct RegionCollector<'a> {
    source: &'a str,
    single_quoted: Vec<TextRange>,
    double_quoted: Vec<TextRange>,
    heredocs: Vec<TextRange>,
    command_substitutions: Vec<TextRange>,
    arithmetic: Vec<TextRange>,
    conditionals: Vec<TextRange>,
    quoted_heredocs: Vec<TextRange>,
}

impl<'a> RegionCollector<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
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
                let range = redirect.target.span.to_range();
                push_range(&mut self.heredocs, range);
                if redirect.target.quoted {
                    push_range(&mut self.quoted_heredocs, range);
                }
                self.visit_word_parts(&redirect.target);
            }
            _ => self.visit_word(&redirect.target, true),
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
        if scan_quotes {
            self.scan_word_quotes(word);
        }
        self.visit_word_parts(word);
    }

    fn visit_word_parts(&mut self, word: &Word) {
        for (part, span) in word.parts_with_spans() {
            let range = span.to_range();
            match part {
                WordPart::CommandSubstitution(commands) => {
                    push_range(&mut self.command_substitutions, range);
                    self.visit_commands(commands);
                }
                WordPart::ArithmeticExpansion(_) => {
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

    fn scan_word_quotes(&mut self, word: &Word) {
        if !valid_span(word.span, self.source) {
            return;
        }

        let opaque_ranges = word
            .parts_with_spans()
            .filter_map(|(part, span)| {
                (!matches!(part, WordPart::Literal(_))).then_some(span.to_range())
            })
            .collect::<Vec<_>>();

        let mut opaque_index = 0usize;
        let mut offset = word.span.start.offset;
        let end = word.span.end.offset;

        while offset < end {
            if let Some(next_offset) = skip_opaque_range(&opaque_ranges, &mut opaque_index, offset)
            {
                offset = next_offset;
                continue;
            }

            let Some((ch, next_offset)) = next_char(self.source, offset) else {
                break;
            };

            match ch {
                '\'' => {
                    let range_end = scan_single_quote(self.source, next_offset, end);
                    push_range(
                        &mut self.single_quoted,
                        TextRange::new(
                            TextSize::new(offset as u32),
                            TextSize::new(range_end as u32),
                        ),
                    );
                    offset = range_end;
                }
                '"' => {
                    let range_end = scan_double_quote(
                        self.source,
                        next_offset,
                        end,
                        &opaque_ranges,
                        &mut opaque_index,
                    );
                    push_range(
                        &mut self.double_quoted,
                        TextRange::new(
                            TextSize::new(offset as u32),
                            TextSize::new(range_end as u32),
                        ),
                    );
                    offset = range_end;
                }
                _ => offset = next_offset,
            }
        }
    }
}

fn sort_ranges(ranges: &mut [TextRange]) {
    ranges.sort_unstable_by_key(|range| (range.start().to_u32(), range.end().to_u32()));
}

fn push_range(ranges: &mut Vec<TextRange>, range: TextRange) {
    if !range.is_empty() {
        ranges.push(range);
    }
}

fn valid_span(span: Span, source: &str) -> bool {
    span.start.offset < span.end.offset && span.end.offset <= source.len()
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

fn skip_opaque_range(ranges: &[TextRange], index: &mut usize, offset: usize) -> Option<usize> {
    while let Some(range) = ranges.get(*index) {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        if end <= offset {
            *index += 1;
            continue;
        }
        if start <= offset {
            return Some(end);
        }
        break;
    }
    None
}

fn scan_single_quote(source: &str, mut offset: usize, end: usize) -> usize {
    while offset < end {
        let Some((ch, next_offset)) = next_char(source, offset) else {
            break;
        };
        offset = next_offset;
        if ch == '\'' {
            break;
        }
    }
    offset
}

fn scan_double_quote(
    source: &str,
    mut offset: usize,
    end: usize,
    opaque_ranges: &[TextRange],
    opaque_index: &mut usize,
) -> usize {
    while offset < end {
        if let Some(next_offset) = skip_opaque_range(opaque_ranges, opaque_index, offset) {
            offset = next_offset;
            continue;
        }

        let Some((ch, next_offset)) = next_char(source, offset) else {
            break;
        };
        match ch {
            '\\' => {
                offset = next_offset;
                if offset < end {
                    offset = next_char(source, offset).map_or(end, |(_, next)| next);
                }
            }
            '"' => return next_offset,
            _ => offset = next_offset,
        }
    }

    offset
}

fn next_char(source: &str, offset: usize) -> Option<(char, usize)> {
    let ch = source.get(offset..)?.chars().next()?;
    Some((ch, offset + ch.len_utf8()))
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
