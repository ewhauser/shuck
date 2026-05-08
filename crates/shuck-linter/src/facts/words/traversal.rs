use super::*;
use shuck_ast::{LiteralText, PatternGroupKind, PatternPartNode};

#[derive(Clone, Copy)]
pub(in crate::facts) struct WordTraversalContext<'a> {
    pub source: &'a str,
    pub locator: Option<Locator<'a>>,
    pub shell_dialect: shuck_semantic::ShellDialect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::facts) enum WordTraversalOrigin {
    Root,
    ArithmeticExpansion,
    ParameterOperand,
    ParameterPattern,
    ZshQualifiedGlobPattern,
    ZshParameterTarget,
    ZshModifierArgument,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::facts) enum WordTraversalPatternContext {
    ParameterOperator,
    ZshQualifiedGlob,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::facts) struct WordTraversalState<'a> {
    pub parent_word_span: Span,
    pub part_index: Option<usize>,
    pub siblings: Option<&'a [WordPartNode]>,
    pub in_double_quote: bool,
    pub origin: WordTraversalOrigin,
    pub pattern_context: Option<WordTraversalPatternContext>,
}

impl<'a> WordTraversalState<'a> {
    fn root(word: &'a Word) -> Self {
        Self {
            parent_word_span: word.span,
            part_index: None,
            siblings: None,
            in_double_quote: false,
            origin: WordTraversalOrigin::Root,
            pattern_context: None,
        }
    }

    pub(in crate::facts) fn processes_root_word(self) -> bool {
        self.origin == WordTraversalOrigin::Root
    }

    fn with_part(
        self,
        _part: &'a WordPartNode,
        siblings: &'a [WordPartNode],
        part_index: usize,
    ) -> Self {
        Self {
            parent_word_span: self.parent_word_span,
            part_index: Some(part_index),
            siblings: Some(siblings),
            in_double_quote: self.in_double_quote,
            origin: self.origin,
            pattern_context: self.pattern_context,
        }
    }

    fn without_part(self) -> Self {
        Self {
            parent_word_span: self.parent_word_span,
            part_index: None,
            siblings: None,
            in_double_quote: self.in_double_quote,
            origin: self.origin,
            pattern_context: self.pattern_context,
        }
    }

    fn inside_double_quote(self) -> Self {
        Self {
            in_double_quote: true,
            ..self.without_part()
        }
    }

    fn with_origin(self, origin: WordTraversalOrigin, word_span: Span) -> Self {
        Self {
            parent_word_span: word_span,
            part_index: None,
            siblings: None,
            in_double_quote: false,
            origin,
            pattern_context: self.pattern_context,
        }
    }

    fn with_pattern_context(self, pattern_context: WordTraversalPatternContext) -> Self {
        Self {
            pattern_context: Some(pattern_context),
            ..self.without_part()
        }
    }
}

pub(in crate::facts) trait WordSubtreeVisitor<'a> {
    fn enter_word(&mut self, _word: &'a Word, _state: WordTraversalState<'a>) {}
    fn exit_word(&mut self, _word: &'a Word, _state: WordTraversalState<'a>) {}
    fn visit_part(&mut self, _part: &'a WordPartNode, _state: WordTraversalState<'a>) {}
    fn visit_literal(
        &mut self,
        _part: &'a WordPartNode,
        _text: &'a str,
        _state: WordTraversalState<'a>,
    ) {
    }
    fn visit_single_quoted(&mut self, _part: &'a WordPartNode, _state: WordTraversalState<'a>) {}
    fn enter_double_quoted(&mut self, _part: &'a WordPartNode, _state: WordTraversalState<'a>) {}
    fn exit_double_quoted(&mut self, _part: &'a WordPartNode, _state: WordTraversalState<'a>) {}
    fn visit_command_substitution(
        &mut self,
        _part: &'a WordPartNode,
        _state: WordTraversalState<'a>,
    ) {
    }
    fn visit_arithmetic_expansion(
        &mut self,
        _part: &'a WordPartNode,
        _state: WordTraversalState<'a>,
    ) {
    }
    fn visit_parameter_expansion(
        &mut self,
        _part: &'a WordPartNode,
        _state: WordTraversalState<'a>,
    ) {
    }
    fn visit_zsh_qualified_glob(
        &mut self,
        _part: &'a WordPartNode,
        _glob: &'a ZshQualifiedGlob,
        _state: WordTraversalState<'a>,
    ) {
    }
    fn visit_pattern_group(
        &mut self,
        _part: &'a PatternPartNode,
        _kind: PatternGroupKind,
        _state: WordTraversalState<'a>,
    ) {
    }
    fn visit_pattern_word(&mut self, _word: &'a Word, _state: WordTraversalState<'a>) {}
    fn visit_pattern_char_class(
        &mut self,
        _part: &'a PatternPartNode,
        _state: WordTraversalState<'a>,
    ) {
    }
}

pub(in crate::facts) fn walk_word_subtree<'a>(
    word: &'a Word,
    context: WordTraversalContext<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    let _ = (context.locator, context.shell_dialect);
    walk_word(word, context, WordTraversalState::root(word), visitor);
}

fn walk_word<'a>(
    word: &'a Word,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    visitor.enter_word(word, state);
    walk_word_parts(&word.parts, context, state, visitor);
    visitor.exit_word(word, state);
}

fn walk_word_parts<'a>(
    parts: &'a [WordPartNode],
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    for (index, part) in parts.iter().enumerate() {
        let part_state = state.with_part(part, parts, index);
        visitor.visit_part(part, part_state);
        match &part.kind {
            WordPart::Literal(text) => {
                visitor.visit_literal(
                    part,
                    literal_text_for_context(text, context.source, part.span),
                    part_state,
                );
            }
            WordPart::SingleQuoted { .. } => visitor.visit_single_quoted(part, part_state),
            WordPart::DoubleQuoted { parts, .. } => {
                visitor.enter_double_quoted(part, part_state);
                walk_word_parts(parts, context, part_state.inside_double_quote(), visitor);
                visitor.exit_double_quoted(part, part_state);
            }
            WordPart::Variable(_)
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
            WordPart::CommandSubstitution { .. } => {
                visitor.visit_command_substitution(part, part_state);
            }
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                visitor.visit_arithmetic_expansion(part, part_state);
                if let Some(expression) = expression_ast.as_deref() {
                    walk_arithmetic_expression_words(
                        expression,
                        context,
                        part_state,
                        WordTraversalOrigin::ArithmeticExpansion,
                        visitor,
                    );
                } else {
                    walk_embedded_word(
                        expression_word_ast,
                        context,
                        part_state,
                        WordTraversalOrigin::ArithmeticExpansion,
                        visitor,
                    );
                }
            }
            WordPart::Parameter(parameter) => {
                visitor.visit_parameter_expansion(part, part_state);
                walk_parameter_expansion(parameter, context, part_state, visitor);
            }
            WordPart::ParameterExpansion {
                operator,
                operand_word_ast,
                ..
            }
            | WordPart::IndirectExpansion {
                operator: Some(operator),
                operand_word_ast,
                ..
            } => {
                visitor.visit_parameter_expansion(part, part_state);
                walk_parameter_operator(operator, context, part_state, visitor);
                if let Some(operand_word) = operand_word_ast.as_deref() {
                    walk_embedded_word(
                        operand_word,
                        context,
                        part_state,
                        WordTraversalOrigin::ParameterOperand,
                        visitor,
                    );
                }
            }
            WordPart::IndirectExpansion { operator: None, .. } => {
                visitor.visit_parameter_expansion(part, part_state);
            }
            WordPart::Substring {
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                walk_arithmetic_operand_word(
                    offset_ast.as_deref(),
                    offset_word_ast,
                    context,
                    part_state,
                    visitor,
                );
                if let Some(length_ast) = length_ast.as_deref() {
                    walk_arithmetic_expression_words(
                        length_ast,
                        context,
                        part_state,
                        WordTraversalOrigin::ParameterOperand,
                        visitor,
                    );
                } else if let Some(length_word) = length_word_ast.as_deref() {
                    walk_embedded_word(
                        length_word,
                        context,
                        part_state,
                        WordTraversalOrigin::ParameterOperand,
                        visitor,
                    );
                }
            }
            WordPart::ZshQualifiedGlob(glob) => {
                visitor.visit_zsh_qualified_glob(part, glob, part_state);
                walk_zsh_qualified_glob(glob, context, part_state, visitor);
            }
        }
    }
}

fn literal_text_for_context<'a>(text: &'a LiteralText, source: &'a str, span: Span) -> &'a str {
    match text {
        LiteralText::Owned(_) => text.as_str(source, span),
        LiteralText::Source | LiteralText::CookedSource(_) if span.end.offset <= source.len() => {
            text.as_str(source, span)
        }
        LiteralText::Source | LiteralText::CookedSource(_) => "",
    }
}

fn walk_embedded_word<'a>(
    word: &'a Word,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    origin: WordTraversalOrigin,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    walk_word(word, context, state.with_origin(origin, word.span), visitor);
}

fn walk_arithmetic_operand_word<'a>(
    expression: Option<&'a ArithmeticExprNode>,
    word: &'a Word,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    if let Some(expression) = expression {
        walk_arithmetic_expression_words(
            expression,
            context,
            state,
            WordTraversalOrigin::ParameterOperand,
            visitor,
        );
    } else {
        walk_embedded_word(
            word,
            context,
            state,
            WordTraversalOrigin::ParameterOperand,
            visitor,
        );
    }
}

fn walk_arithmetic_expression_words<'a>(
    expression: &'a ArithmeticExprNode,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    origin: WordTraversalOrigin,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    visit_arithmetic_words(expression, &mut |word| {
        walk_embedded_word(word, context, state, origin, visitor);
    });
}

fn walk_parameter_expansion<'a>(
    parameter: &'a ParameterExpansion,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => {
            walk_bourne_parameter_expansion(syntax, context, state, visitor);
        }
        ParameterExpansionSyntax::Zsh(syntax) => {
            walk_zsh_parameter_expansion(syntax, context, state, visitor);
        }
    }
}

fn walk_bourne_parameter_expansion<'a>(
    syntax: &'a BourneParameterExpansion,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    match syntax {
        BourneParameterExpansion::Operation {
            operator,
            operand_word_ast,
            ..
        }
        | BourneParameterExpansion::Indirect {
            operator: Some(operator),
            operand_word_ast,
            ..
        } => {
            walk_parameter_operator(operator, context, state, visitor);
            if let Some(operand_word) = operand_word_ast.as_deref() {
                walk_embedded_word(
                    operand_word,
                    context,
                    state,
                    WordTraversalOrigin::ParameterOperand,
                    visitor,
                );
            }
        }
        BourneParameterExpansion::Slice {
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        } => {
            walk_arithmetic_operand_word(
                offset_ast.as_deref(),
                offset_word_ast,
                context,
                state,
                visitor,
            );
            if let Some(length_ast) = length_ast.as_deref() {
                walk_arithmetic_expression_words(
                    length_ast,
                    context,
                    state,
                    WordTraversalOrigin::ParameterOperand,
                    visitor,
                );
            } else if let Some(length_word) = length_word_ast.as_deref() {
                walk_embedded_word(
                    length_word,
                    context,
                    state,
                    WordTraversalOrigin::ParameterOperand,
                    visitor,
                );
            }
        }
        BourneParameterExpansion::Access { .. }
        | BourneParameterExpansion::Length { .. }
        | BourneParameterExpansion::Indices { .. }
        | BourneParameterExpansion::Indirect { operator: None, .. }
        | BourneParameterExpansion::PrefixMatch { .. }
        | BourneParameterExpansion::Transformation { .. } => {}
    }
}

fn walk_parameter_operator<'a>(
    operator: &'a ParameterOp,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => {
            walk_pattern(
                pattern,
                context,
                state.with_pattern_context(WordTraversalPatternContext::ParameterOperator),
                WordTraversalOrigin::ParameterPattern,
                visitor,
            );
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            pattern,
            replacement_word_ast,
            ..
        } => {
            walk_pattern(
                pattern,
                context,
                state.with_pattern_context(WordTraversalPatternContext::ParameterOperator),
                WordTraversalOrigin::ParameterPattern,
                visitor,
            );
            walk_embedded_word(
                replacement_word_ast,
                context,
                state,
                WordTraversalOrigin::ParameterOperand,
                visitor,
            );
        }
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

fn walk_zsh_parameter_expansion<'a>(
    syntax: &'a ZshParameterExpansion,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    match &syntax.target {
        ZshExpansionTarget::Nested(parameter) => {
            walk_parameter_expansion(parameter, context, state, visitor);
        }
        ZshExpansionTarget::Word(word) => {
            walk_embedded_word(
                word,
                context,
                state,
                WordTraversalOrigin::ZshParameterTarget,
                visitor,
            );
        }
        ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Empty => {}
    }

    for modifier in &syntax.modifiers {
        if let Some(word) = modifier.argument_word_ast() {
            walk_embedded_word(
                word,
                context,
                state,
                WordTraversalOrigin::ZshModifierArgument,
                visitor,
            );
        }
    }

    if let Some(operation) = syntax.operation.as_ref() {
        walk_zsh_expansion_operation(operation, context, state, visitor);
    }
}

fn walk_zsh_expansion_operation<'a>(
    operation: &'a ZshExpansionOperation,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    if let Some(word) = operation.operand_word_ast() {
        walk_embedded_word(
            word,
            context,
            state,
            WordTraversalOrigin::ParameterOperand,
            visitor,
        );
    }
    if let Some(word) = operation.pattern_word_ast() {
        walk_embedded_word(
            word,
            context,
            state,
            WordTraversalOrigin::ParameterPattern,
            visitor,
        );
    }
    if let Some(word) = operation.replacement_word_ast() {
        walk_embedded_word(
            word,
            context,
            state,
            WordTraversalOrigin::ParameterOperand,
            visitor,
        );
    }
    if let Some(word) = operation.offset_word_ast() {
        walk_embedded_word(
            word,
            context,
            state,
            WordTraversalOrigin::ParameterOperand,
            visitor,
        );
    }
    if let Some(word) = operation.length_word_ast() {
        walk_embedded_word(
            word,
            context,
            state,
            WordTraversalOrigin::ParameterOperand,
            visitor,
        );
    }
}

fn walk_zsh_qualified_glob<'a>(
    glob: &'a ZshQualifiedGlob,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    let state = state.with_pattern_context(WordTraversalPatternContext::ZshQualifiedGlob);
    for segment in &glob.segments {
        if let ZshGlobSegment::Pattern(pattern) = segment {
            walk_pattern(
                pattern,
                context,
                state,
                WordTraversalOrigin::ZshQualifiedGlobPattern,
                visitor,
            );
        }
    }
}

fn walk_pattern<'a>(
    pattern: &'a Pattern,
    context: WordTraversalContext<'a>,
    state: WordTraversalState<'a>,
    word_origin: WordTraversalOrigin,
    visitor: &mut impl WordSubtreeVisitor<'a>,
) {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Group { kind, patterns } => {
                visitor.visit_pattern_group(part, *kind, state);
                for pattern in patterns {
                    walk_pattern(pattern, context, state, word_origin, visitor);
                }
            }
            PatternPart::Word(word) => {
                let word_state = state.with_origin(word_origin, word.span);
                visitor.visit_pattern_word(word, word_state);
                walk_word(word, context, word_state, None, visitor);
            }
            PatternPart::CharClass(_) => visitor.visit_pattern_char_class(part, state),
            PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_indexer::LineIndex;
    use shuck_parser::parser::Parser;

    #[derive(Default)]
    struct RecordingVisitor {
        events: Vec<String>,
    }

    impl<'a> WordSubtreeVisitor<'a> for RecordingVisitor {
        fn visit_literal(
            &mut self,
            part: &'a WordPartNode,
            text: &'a str,
            state: WordTraversalState<'a>,
        ) {
            self.events.push(format!(
                "literal:{text}:{:?}:{:?}",
                state.origin, state.in_double_quote
            ));
            let siblings = state.siblings.expect("literal should expose siblings");
            let part_index = state.part_index.expect("literal should expose index");
            assert_eq!(siblings[part_index].span, part.span);
        }

        fn enter_double_quoted(&mut self, _part: &'a WordPartNode, _state: WordTraversalState<'a>) {
            self.events.push("enter-double".to_owned());
        }

        fn visit_command_substitution(
            &mut self,
            _part: &'a WordPartNode,
            state: WordTraversalState<'a>,
        ) {
            self.events.push(format!(
                "command:{:?}:{:?}",
                state.origin, state.in_double_quote
            ));
        }

        fn visit_arithmetic_expansion(
            &mut self,
            _part: &'a WordPartNode,
            state: WordTraversalState<'a>,
        ) {
            self.events.push(format!("arithmetic:{:?}", state.origin));
        }

        fn visit_parameter_expansion(
            &mut self,
            _part: &'a WordPartNode,
            state: WordTraversalState<'a>,
        ) {
            self.events.push(format!("parameter:{:?}", state.origin));
        }

        fn visit_pattern_group(
            &mut self,
            _part: &'a PatternPartNode,
            kind: PatternGroupKind,
            state: WordTraversalState<'a>,
        ) {
            self.events.push(format!(
                "pattern-group:{kind:?}:{:?}",
                state.pattern_context
            ));
        }

        fn visit_pattern_word(&mut self, _word: &'a Word, state: WordTraversalState<'a>) {
            self.events.push(format!("pattern-word:{:?}", state.origin));
        }

        fn visit_zsh_qualified_glob(
            &mut self,
            _part: &'a WordPartNode,
            _glob: &'a ZshQualifiedGlob,
            state: WordTraversalState<'a>,
        ) {
            self.events.push(format!("zsh-glob:{:?}", state.origin));
        }
    }

    fn first_arg(source: &str) -> Word {
        let output = Parser::new(source).parse().unwrap();
        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        command.args[0].clone()
    }

    fn walk_recorded(source: &str, word: &Word) -> RecordingVisitor {
        let line_index = LineIndex::new(source);
        let context = WordTraversalContext {
            source,
            locator: Some(Locator::new(source, &line_index)),
            shell_dialect: shuck_semantic::ShellDialect::Bash,
        };
        let mut visitor = RecordingVisitor::default();
        walk_word_subtree(word, context, &mut visitor);
        visitor
    }

    #[test]
    fn walker_visits_literals_and_double_quoted_nested_parts() {
        let source = "echo pre\"$value $(date)\"post\n";
        let word = first_arg(source);
        let visitor = walk_recorded(source, &word);

        assert!(
            visitor
                .events
                .iter()
                .any(|event| event == "literal:pre:Root:false")
        );
        assert!(visitor.events.iter().any(|event| event == "enter-double"));
        assert!(
            visitor
                .events
                .iter()
                .any(|event| event == "command:Root:true")
        );
    }

    #[test]
    fn walker_visits_parameter_operand_and_pattern_words() {
        let source = "echo ${name:-$(fallback)} ${path#@(tmp|var)/$leaf}\n";
        let first = first_arg(source);
        let second = {
            let output = Parser::new(source).parse().unwrap();
            let Command::Simple(command) = &output.file.body[0].command else {
                panic!("expected simple command");
            };
            command.args[1].clone()
        };

        let operand = walk_recorded(source, &first);
        assert!(
            operand
                .events
                .iter()
                .any(|event| event == "command:ParameterOperand:false")
        );

        let pattern = walk_recorded(source, &second);
        assert!(
            pattern
                .events
                .iter()
                .any(|event| event == "pattern-group:ExactlyOne:Some(ParameterOperator)")
        );
        assert!(
            pattern
                .events
                .iter()
                .any(|event| event == "pattern-word:ParameterPattern")
        );
    }

    #[test]
    fn walker_visits_arithmetic_word_asts() {
        let source = "echo $(( $(value) + 1 ))\n";
        let word = first_arg(source);
        let visitor = walk_recorded(source, &word);

        assert!(
            visitor
                .events
                .iter()
                .any(|event| event == "arithmetic:Root")
        );
        assert!(
            visitor
                .events
                .iter()
                .any(|event| event == "command:ArithmeticExpansion:false")
        );
    }

    #[test]
    fn walker_visits_zsh_qualified_glob_pattern_words() {
        let source = "prefix";
        let inner = Word::literal_with_span("prefix", span_for(source, 0, source.len()));
        let pattern = Pattern {
            parts: vec![PatternPartNode::new(
                PatternPart::Word(inner),
                span_for(source, 0, source.len()),
            )],
            span: span_for(source, 0, source.len()),
        };
        let word = Word {
            parts: vec![WordPartNode::new(
                WordPart::ZshQualifiedGlob(ZshQualifiedGlob {
                    span: span_for(source, 0, source.len()),
                    segments: vec![ZshGlobSegment::Pattern(pattern)],
                    qualifiers: None,
                }),
                span_for(source, 0, source.len()),
            )],
            span: span_for(source, 0, source.len()),
            brace_syntax: Vec::new(),
        };
        let visitor = walk_recorded(source, &word);

        assert!(visitor.events.iter().any(|event| event == "zsh-glob:Root"));
        assert!(
            visitor
                .events
                .iter()
                .any(|event| event == "pattern-word:ZshQualifiedGlobPattern")
        );
    }

    fn span_for(source: &str, start: usize, end: usize) -> Span {
        let start = Position::new().advanced_by(&source[..start]);
        let end = Position::new().advanced_by(&source[..end]);
        Span::from_positions(start, end)
    }
}
