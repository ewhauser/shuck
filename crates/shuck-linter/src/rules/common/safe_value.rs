use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    Name, ParameterOp, RedirectKind, SourceText, Span, VarRef, Word, WordPart, WordPartNode,
};
use shuck_semantic::BindingId;
use shuck_semantic::{BindingAttributes, BindingKind, SemanticModel};

use crate::{FactSpan, LinterFacts};

use super::{
    expansion::{ExpansionContext, analyze_word},
    word::{classify_contextual_operand, static_word_text},
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
    source: &'a str,
    scalar_bindings: FxHashMap<FactSpan, &'a Word>,
    maybe_uninitialized_refs: FxHashSet<FactSpan>,
    memo: FxHashMap<(FactSpan, SafeValueQuery), bool>,
    visiting: FxHashSet<(FactSpan, SafeValueQuery)>,
}

impl<'a> SafeValueIndex<'a> {
    pub fn build(semantic: &'a SemanticModel, facts: &LinterFacts<'a>, source: &'a str) -> Self {
        let maybe_uninitialized_refs = semantic
            .uninitialized_references()
            .iter()
            .map(|uninitialized| FactSpan::new(semantic.reference(uninitialized.reference).span))
            .collect();

        Self {
            semantic,
            source,
            scalar_bindings: facts.scalar_binding_values().clone(),
            maybe_uninitialized_refs,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    pub fn part_is_safe(&mut self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        match part {
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
                name,
                operator,
                operand,
                ..
            } => {
                self.indirect_name_is_safe(name, span, query)
                    && operator.as_ref().is_none_or(|operator| {
                        self.parameter_operator_is_safe(
                            name,
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
        let analysis = analyze_word(word, self.source);
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
            && !classify_contextual_operand(&word, self.source, context).is_fixed_literal()
        {
            return false;
        }

        static_word_text(&word, self.source).is_some_and(|text| query.literal_is_safe(&text))
    }

    fn name_is_safe(&mut self, name: &Name, at: Span, query: SafeValueQuery) -> bool {
        if self.maybe_uninitialized_refs.contains(&FactSpan::new(at)) {
            return false;
        }
        if safe_special_parameter(name) {
            return true;
        }

        let Some(binding) = self.semantic.visible_binding(name, at) else {
            return false;
        };
        self.binding_is_safe(binding.id, query)
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

    fn indirect_name_is_safe(&mut self, name: &Name, at: Span, query: SafeValueQuery) -> bool {
        if self.maybe_uninitialized_refs.contains(&FactSpan::new(at)) {
            return false;
        }

        let Some(binding) = self.semantic.visible_binding(name, at) else {
            return false;
        };
        let targets = self.semantic.indirect_targets_for_binding(binding.id);
        !targets.is_empty()
            && targets
                .iter()
                .copied()
                .all(|target| self.binding_is_safe(target, query))
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

#[cfg(test)]
mod tests {
    use shuck_ast::Command;
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use shuck_semantic::SemanticModel;

    use super::{SafeValueIndex, SafeValueQuery};
    use crate::LinterFacts;
    use crate::rules::common::{expansion::ExpansionContext, query};
    use crate::{ShellDialect, classify_file_context};

    #[test]
    fn maps_pattern_and_regex_contexts_into_safe_value_queries() {
        use shuck_ast::RedirectKind;

        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CommandArgument),
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
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(safe_values.word_is_safe(&command.args[1], SafeValueQuery::Quoted));
        assert!(!safe_values.word_is_safe(&command.args[2], SafeValueQuery::Argv));
    }

    #[test]
    fn distinguishes_pattern_and_regex_safe_bindings() {
        let source = "\
#!/bin/bash
plain=abc
glob='*.sh'
regex='a+'
case $value in
  $plain) : ;;
  $glob) : ;;
esac
[[ $value =~ $plain ]]
[[ $value =~ $regex ]]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &facts, source);

        let words = query::iter_commands(&output.file.body, query::CommandWalkOptions::default())
            .flat_map(|visit| query::iter_expansion_words(visit, source))
            .collect::<Vec<_>>();

        let pattern_plain = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::CasePattern && word.span.slice(source) == "$plain")
                    .then_some(word)
            })
            .expect("expected plain case-pattern word");
        let pattern_glob = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::CasePattern && word.span.slice(source) == "$glob")
                    .then_some(word)
            })
            .expect("expected glob case-pattern word");
        let regex_plain = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::RegexOperand && word.span.slice(source) == "$plain")
                    .then_some(word)
            })
            .expect("expected plain regex word");
        let regex_runtime = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::RegexOperand && word.span.slice(source) == "$regex")
                    .then_some(word)
            })
            .expect("expected runtime regex word");

        assert!(safe_values.word_is_safe(pattern_plain, SafeValueQuery::Pattern));
        assert!(!safe_values.word_is_safe(pattern_glob, SafeValueQuery::Pattern));
        assert!(safe_values.word_is_safe(regex_plain, SafeValueQuery::Regex));
        assert!(!safe_values.word_is_safe(regex_runtime, SafeValueQuery::Regex));
    }

    #[test]
    fn supports_default_trim_and_replacement_parameter_operators() {
        let source = "\
#!/bin/bash
base=abc
fallback=${base:-safe}
trimmed=${base#?}
replaced=${base/b/x}
unsafe=${base:+a b}
printf '%s\\n' $fallback $trimmed $replaced $unsafe
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &facts, source);

        let words = query::iter_commands(&output.file.body, query::CommandWalkOptions::default())
            .flat_map(|visit| query::iter_expansion_words(visit, source))
            .collect::<Vec<_>>();

        let fallback = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::CommandArgument
                    && word.span.slice(source) == "$fallback")
                    .then_some(word)
            })
            .expect("expected fallback argument");
        let trimmed = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::CommandArgument
                    && word.span.slice(source) == "$trimmed")
                    .then_some(word)
            })
            .expect("expected trimmed argument");
        let replaced = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::CommandArgument
                    && word.span.slice(source) == "$replaced")
                    .then_some(word)
            })
            .expect("expected replaced argument");
        let unsafe_replacement = words
            .iter()
            .find_map(|(word, context)| {
                (*context == ExpansionContext::CommandArgument
                    && word.span.slice(source) == "$unsafe")
                    .then_some(word)
            })
            .expect("expected unsafe argument");

        assert!(safe_values.word_is_safe(fallback, SafeValueQuery::Argv));
        assert!(safe_values.word_is_safe(trimmed, SafeValueQuery::Argv));
        assert!(safe_values.word_is_safe(replaced, SafeValueQuery::Argv));
        assert!(!safe_values.word_is_safe(unsafe_replacement, SafeValueQuery::Argv));
    }
}
