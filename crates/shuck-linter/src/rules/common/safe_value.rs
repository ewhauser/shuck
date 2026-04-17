use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    BourneParameterExpansion, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp,
    RedirectKind, SourceText, Span, VarRef, Word, WordPart, WordPartNode,
};
use shuck_semantic::BindingId;
use shuck_semantic::{BindingAttributes, BindingKind, SemanticAnalysis, SemanticModel};

use crate::{FactSpan, LinterFacts};

use super::{
    expansion::{ExpansionContext, analyze_literal_runtime},
    word::static_word_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SafeValueQuery {
    Argv,
    RedirectTarget,
    Pattern,
    Regex,
    Quoted,
}

impl SafeValueQuery {
    pub fn from_context(context: ExpansionContext) -> Option<Self> {
        match context {
            ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::HereString
            | ExpansionContext::DeclarationAssignmentValue => Some(Self::Argv),
            ExpansionContext::RedirectTarget(_) => Some(Self::RedirectTarget),
            ExpansionContext::CasePattern
            | ExpansionContext::ConditionalPattern
            | ExpansionContext::ParameterPattern => Some(Self::Pattern),
            ExpansionContext::RegexOperand => Some(Self::Regex),
            _ => None,
        }
    }

    fn operand_context(self) -> Option<ExpansionContext> {
        match self {
            Self::Argv => Some(ExpansionContext::CommandArgument),
            Self::RedirectTarget => Some(ExpansionContext::RedirectTarget(RedirectKind::Output)),
            Self::Pattern => Some(ExpansionContext::CasePattern),
            Self::Regex => Some(ExpansionContext::RegexOperand),
            Self::Quoted => None,
        }
    }

    fn literal_is_safe(self, text: &str) -> bool {
        match self {
            Self::Argv | Self::RedirectTarget => literal_is_field_safe(text),
            Self::Pattern => literal_is_pattern_safe(text),
            Self::Regex => literal_is_regex_safe(text),
            Self::Quoted => true,
        }
    }
}

pub struct SafeValueIndex<'a> {
    semantic: &'a SemanticModel,
    analysis: &'a SemanticAnalysis<'a>,
    facts: &'a LinterFacts<'a>,
    source: &'a str,
    scalar_bindings: FxHashMap<FactSpan, &'a Word>,
    loop_bindings: FxHashMap<FactSpan, Box<[&'a Word]>>,
    maybe_uninitialized_refs: FxHashSet<FactSpan>,
    memo: FxHashMap<(FactSpan, SafeValueQuery), bool>,
    visiting: FxHashSet<(FactSpan, SafeValueQuery)>,
}

impl<'a> SafeValueIndex<'a> {
    pub fn build(
        semantic: &'a SemanticModel,
        analysis: &'a SemanticAnalysis<'a>,
        facts: &'a LinterFacts<'a>,
        source: &'a str,
    ) -> Self {
        let maybe_uninitialized_refs = analysis
            .uninitialized_references()
            .iter()
            .map(|uninitialized| FactSpan::new(semantic.reference(uninitialized.reference).span))
            .collect();

        Self {
            semantic,
            analysis,
            facts,
            source,
            scalar_bindings: facts.scalar_binding_values().clone(),
            loop_bindings: facts.loop_binding_value_sets().clone(),
            maybe_uninitialized_refs,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    pub fn part_is_safe(&mut self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        match part {
            WordPart::ZshQualifiedGlob(_) => query == SafeValueQuery::Quoted,
            WordPart::Parameter(parameter) => self.parameter_part_is_safe(parameter, span, query),
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                self.literal_part_is_safe(part, span, query)
            }
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter()
                .all(|part| self.part_is_safe(&part.kind, part.span, query)),
            WordPart::Variable(name) => self.name_is_safe(name, span, query),
            WordPart::ArithmeticExpansion { .. } => true,
            WordPart::Length(_) | WordPart::ArrayLength(_) => true,
            WordPart::ArrayAccess(reference) => {
                (query == SafeValueQuery::Quoted || !reference.has_array_selector())
                    && self.reference_is_safe(reference, span, query)
            }
            WordPart::Substring { reference, .. } => self.reference_is_safe(reference, span, query),
            WordPart::Transformation {
                reference,
                operator,
            } => self.transformation_is_safe(reference, *operator, span, query),
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                ..
            } => {
                self.indirect_name_is_safe(reference, span, query)
                    && operator.as_ref().is_none_or(|operator| {
                        self.parameter_operator_is_safe(
                            &reference.name,
                            operator,
                            operand.as_ref(),
                            span,
                            query,
                        )
                    })
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. } => query == SafeValueQuery::Quoted,
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                ..
            } => {
                self.parameter_expansion_is_safe(reference, operator, operand.as_ref(), span, query)
            }
        }
    }

    pub fn word_is_safe(&mut self, word: &Word, query: SafeValueQuery) -> bool {
        let Some(analysis) = self
            .facts
            .any_word_fact(word.span)
            .map(|fact| fact.analysis())
        else {
            return false;
        };
        if query != SafeValueQuery::Quoted
            && (analysis.array_valued || analysis.hazards.command_or_process_substitution)
        {
            return false;
        }

        word.parts_with_spans()
            .all(|(part, span)| self.part_is_safe(part, span, query))
    }

    fn literal_part_is_safe(&self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        let word = Word {
            parts: vec![WordPartNode::new(part.clone(), span)],
            span,
            brace_syntax: Vec::new(),
        };
        if let Some(context) = query.operand_context()
            && analyze_literal_runtime(&word, self.source, context, None).is_runtime_sensitive()
        {
            return false;
        }

        static_word_text(&word, self.source).is_some_and(|text| query.literal_is_safe(&text))
    }

    fn name_is_safe(&mut self, name: &Name, at: Span, query: SafeValueQuery) -> bool {
        if safe_special_parameter(name) || safe_numeric_shell_variable(name) {
            return true;
        }

        let bindings = self.safe_bindings_for_name(name, at);
        if bindings.is_empty() {
            return false;
        }

        bindings
            .into_iter()
            .all(|binding_id| self.binding_is_safe(binding_id, query))
    }

    fn binding_is_safe(&mut self, binding_id: BindingId, query: SafeValueQuery) -> bool {
        let binding = self.semantic.binding(binding_id);
        if binding.attributes.contains(BindingAttributes::INTEGER)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        let binding_key = FactSpan::new(binding.span);
        let key = (binding_key, query);
        if let Some(result) = self.memo.get(&key) {
            return *result;
        }
        if !self.visiting.insert(key) {
            return false;
        }

        let result = if let Some(word) = self.scalar_bindings.get(&binding_key).copied() {
            self.word_is_safe(word, query)
        } else if let Some(words) = self.loop_bindings.get(&binding_key) {
            let words = words.iter().copied().collect::<Vec<_>>();
            !words.is_empty() && words.into_iter().all(|word| self.word_is_safe(word, query))
        } else {
            false
        };

        self.visiting.remove(&key);
        self.memo.insert(key, result);
        result
    }

    fn reference_is_safe(&mut self, reference: &VarRef, at: Span, query: SafeValueQuery) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }
        self.name_is_safe(&reference.name, at, query)
    }

    fn indirect_name_is_safe(
        &mut self,
        reference: &VarRef,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }
        if self.maybe_uninitialized_refs.contains(&FactSpan::new(at)) {
            return false;
        }

        let bindings = self.safe_bindings_for_name(&reference.name, at);
        if bindings.is_empty() {
            return false;
        }

        bindings.into_iter().all(|binding_id| {
            let targets = self.semantic.indirect_targets_for_binding(binding_id);
            !targets.is_empty()
                && targets
                    .iter()
                    .copied()
                    .all(|target| self.binding_is_safe(target, query))
        })
    }

    fn safe_bindings_for_name(&self, name: &Name, at: Span) -> Vec<BindingId> {
        let mut bindings = self.analysis.reaching_bindings_for_name(name, at);
        if bindings.len() == 1 {
            let mut expanded = self
                .analysis
                .visible_bindings_bypassing(name, bindings[0], at);
            if !expanded.is_empty() {
                expanded.push(bindings[0]);
                expanded
                    .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
                expanded.dedup();
                bindings = expanded;
            }
        }

        bindings
    }

    fn transformation_is_safe(
        &mut self,
        reference: &VarRef,
        operator: char,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }

        match operator {
            'Q' | 'K' | 'k' => true,
            _ => self.reference_is_safe(reference, at, query),
        }
    }

    fn parameter_part_is_safe(
        &mut self,
        parameter: &ParameterExpansion,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference } => {
                    (query == SafeValueQuery::Quoted || !reference.has_array_selector())
                        && self.reference_is_safe(reference, at, query)
                }
                BourneParameterExpansion::Length { .. } => true,
                BourneParameterExpansion::Indices { .. }
                | BourneParameterExpansion::PrefixMatch { .. } => query == SafeValueQuery::Quoted,
                BourneParameterExpansion::Indirect {
                    reference,
                    operator,
                    operand,
                    ..
                } => {
                    self.indirect_name_is_safe(reference, at, query)
                        && operator.as_ref().is_none_or(|operator| {
                            self.parameter_operator_is_safe(
                                &reference.name,
                                operator,
                                operand.as_ref(),
                                at,
                                query,
                            )
                        })
                }
                BourneParameterExpansion::Slice { reference, .. } => {
                    if reference.has_array_selector() {
                        query == SafeValueQuery::Quoted
                    } else {
                        self.reference_is_safe(reference, at, query)
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    ..
                } => self.parameter_expansion_is_safe(
                    reference,
                    operator,
                    operand.as_ref(),
                    at,
                    query,
                ),
                BourneParameterExpansion::Transformation {
                    reference,
                    operator,
                } => self.transformation_is_safe(reference, *operator, at, query),
            },
            ParameterExpansionSyntax::Zsh(_) => false,
        }
    }

    fn parameter_expansion_is_safe(
        &mut self,
        reference: &VarRef,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }

        self.parameter_operator_is_safe(&reference.name, operator, operand, at, query)
    }

    fn parameter_operator_is_safe(
        &mut self,
        name: &Name,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        match operator {
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll
            | ParameterOp::RemovePrefixShort { .. }
            | ParameterOp::RemovePrefixLong { .. }
            | ParameterOp::RemoveSuffixShort { .. }
            | ParameterOp::RemoveSuffixLong { .. } => self.name_is_safe(name, at, query),
            ParameterOp::UseDefault | ParameterOp::AssignDefault | ParameterOp::Error => {
                self.name_is_safe(name, at, query)
                    && operand
                        .is_some_and(|operand| self.source_text_is_safe_literal(operand, query))
            }
            ParameterOp::UseReplacement => {
                operand.is_some_and(|operand| self.source_text_is_safe_literal(operand, query))
            }
            ParameterOp::ReplaceFirst { replacement, .. }
            | ParameterOp::ReplaceAll { replacement, .. } => {
                self.name_is_safe(name, at, query)
                    && self.source_text_is_safe_literal(replacement, query)
            }
        }
    }

    fn source_text_is_safe_literal(&self, text: &SourceText, query: SafeValueQuery) -> bool {
        let text = text.slice(self.source);
        !source_text_needs_parse(text) && query.literal_is_safe(text)
    }
}

fn literal_is_field_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '*' | '?' | '['))
}

fn literal_is_pattern_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | ']' | '|' | '(' | ')'))
}

fn literal_is_regex_safe(text: &str) -> bool {
    let mut escaped = false;

    for character in text.chars() {
        if escaped {
            return false;
        }

        if character == '\\' {
            escaped = true;
            continue;
        }

        if matches!(
            character,
            '.' | '[' | ']' | '(' | ')' | '{' | '}' | '*' | '+' | '?' | '|' | '^' | '$'
        ) {
            return false;
        }
    }

    !escaped
}

fn source_text_needs_parse(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '$' | '`' | '\\' | '\'' | '"'))
}

fn safe_special_parameter(name: &Name) -> bool {
    matches!(name.as_str(), "@" | "#" | "?" | "$" | "!" | "-")
}

fn safe_numeric_shell_variable(name: &Name) -> bool {
    matches!(name.as_str(), "PPID")
}

#[cfg(test)]
mod tests {
    use shuck_ast::Command;
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use shuck_semantic::SemanticModel;

    use super::{SafeValueIndex, SafeValueQuery};
    use crate::LinterFacts;
    use crate::rules::common::expansion::ExpansionContext;
    use crate::{ShellDialect, classify_file_context};

    #[test]
    fn maps_pattern_and_regex_contexts_into_safe_value_queries() {
        use shuck_ast::RedirectKind;

        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CommandArgument),
            Some(SafeValueQuery::Argv)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::HereString),
            Some(SafeValueQuery::Argv)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CommandName),
            Some(SafeValueQuery::Argv)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::RedirectTarget(RedirectKind::Output)),
            Some(SafeValueQuery::RedirectTarget)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CasePattern),
            Some(SafeValueQuery::Pattern)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::ConditionalPattern),
            Some(SafeValueQuery::Pattern)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::ParameterPattern),
            Some(SafeValueQuery::Pattern)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::RegexOperand),
            Some(SafeValueQuery::Regex)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::StringTestOperand),
            None
        );
    }

    #[test]
    fn quoted_query_treats_prefix_matches_as_safe_only_when_quoted() {
        let source = "#!/bin/bash\nprintf '%s\\n' \"${!HOME@}\" ${!HOME@}\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(safe_values.word_is_safe(&command.args[1], SafeValueQuery::Quoted));
        assert!(!safe_values.word_is_safe(&command.args[2], SafeValueQuery::Argv));
    }

    #[test]
    fn treats_zsh_parameter_modifiers_as_dynamic_unknown_values() {
        let source = "print ${(m)foo}\n";
        let output = Parser::with_dialect(source, shuck_parser::parser::ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Zsh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Quoted));
    }

    #[test]
    fn keeps_typed_zsh_parameter_operations_conservative() {
        let source = "print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}}\n";
        let output = Parser::with_dialect(source, shuck_parser::parser::ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Zsh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(
            command
                .args
                .iter()
                .all(|word| !safe_values.word_is_safe(word, SafeValueQuery::Argv))
        );
        assert!(
            command
                .args
                .iter()
                .all(|word| !safe_values.word_is_safe(word, SafeValueQuery::Quoted))
        );
    }

    #[test]
    fn conditional_safe_fallbacks_do_not_hide_unsafe_bindings() {
        let source = "\
#!/bin/bash
foo=$(printf '%s' \"$1\")
if [ \"$foo\" = \"\" ]; then foo=0; fi
[ $foo -eq 1 ]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[2].command else {
            panic!("expected simple test command");
        };

        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn unconditional_safe_overwrites_stay_safe() {
        let source = "\
#!/bin/bash
foo=$(printf '%s' \"$1\")
foo=0
[ $foo -eq 1 ]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[2].command else {
            panic!("expected simple test command");
        };

        assert!(safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn case_arm_safe_overwrites_stay_safe() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
    settings)
        foo=0
        [ $foo -eq 1 ]
        ;;
esac
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Compound(shuck_ast::CompoundCommand::Case(case_command)) =
            &output.file.body[1].command
        else {
            panic!("expected case command");
        };
        let Command::Simple(command) = &case_command.cases[0].body[1].command else {
            panic!("expected simple test command");
        };

        assert!(safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }
}
