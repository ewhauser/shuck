use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    AssignmentValue, DeclOperand, Name, ParameterOp, Span, SubscriptSelector, VarRef, Word,
    WordPart, WordPartNode,
};
use shuck_semantic::BindingId;
use shuck_semantic::{BindingAttributes, BindingKind, SemanticModel};

use crate::rules::common::{
    expansion::{ExpansionContext, analyze_word},
    query::{self, CommandWalkOptions},
    span,
    word::{classify_contextual_operand, classify_word, static_word_text},
};
use crate::{Checker, Rule, Violation};

pub struct UnquotedExpansion;

impl Violation for UnquotedExpansion {
    fn rule() -> Rule {
        Rule::UnquotedExpansion
    }

    fn message(&self) -> String {
        "quote parameter expansions to avoid word splitting and globbing".to_owned()
    }
}

pub fn unquoted_expansion(checker: &mut Checker) {
    let source = checker.source();
    let indexer = checker.indexer();
    let mut safe_values = SafeValueIndex::build(
        checker.semantic(),
        checker.ast().commands.as_slice(),
        source,
    );

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, source, &mut |word, context| {
                if !matches!(
                    context,
                    ExpansionContext::CommandArgument | ExpansionContext::RedirectTarget(_)
                ) {
                    return;
                }

                report_word_expansions(checker, indexer, &mut safe_values, word, context, source);
            });
        },
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SpanKey {
    start: usize,
    end: usize,
}

impl SpanKey {
    fn new(span: Span) -> Self {
        Self {
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SafeValueQuery {
    CommandArgument,
    RedirectTarget,
}

impl SafeValueQuery {
    fn from_context(context: ExpansionContext) -> Self {
        match context {
            ExpansionContext::CommandArgument => Self::CommandArgument,
            ExpansionContext::RedirectTarget(_) => Self::RedirectTarget,
            _ => unreachable!("unsupported safe-value query context"),
        }
    }
}

struct SafeValueIndex<'a> {
    semantic: &'a SemanticModel,
    source: &'a str,
    scalar_bindings: FxHashMap<SpanKey, &'a Word>,
    maybe_uninitialized_refs: FxHashSet<SpanKey>,
    memo: FxHashMap<(SpanKey, SafeValueQuery), bool>,
    visiting: FxHashSet<(SpanKey, SafeValueQuery)>,
}

impl<'a> SafeValueIndex<'a> {
    fn build(
        semantic: &'a SemanticModel,
        commands: &'a [shuck_ast::Command],
        source: &'a str,
    ) -> Self {
        let mut scalar_bindings = FxHashMap::default();

        for visit in query::iter_commands(
            commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        ) {
            for assignment in query::command_assignments(visit.command) {
                let AssignmentValue::Scalar(word) = &assignment.value else {
                    continue;
                };
                scalar_bindings.insert(SpanKey::new(assignment.target.name_span), word);
            }

            for operand in query::declaration_operands(visit.command) {
                let DeclOperand::Assignment(assignment) = operand else {
                    continue;
                };
                let AssignmentValue::Scalar(word) = &assignment.value else {
                    continue;
                };
                scalar_bindings.insert(SpanKey::new(assignment.target.name_span), word);
            }
        }

        let maybe_uninitialized_refs = semantic
            .uninitialized_references()
            .iter()
            .map(|uninitialized| SpanKey::new(semantic.reference(uninitialized.reference).span))
            .collect();

        Self {
            semantic,
            source,
            scalar_bindings,
            maybe_uninitialized_refs,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    fn part_is_field_safe(
        &mut self,
        part: &WordPart,
        span: Span,
        context: ExpansionContext,
    ) -> bool {
        match part {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                self.literal_part_is_field_safe(part, span, context)
            }
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter()
                .all(|part| self.part_is_field_safe(&part.kind, part.span, context)),
            WordPart::Variable(name) => self.name_is_field_safe(name, span, context),
            WordPart::ArithmeticExpansion { .. } => true,
            WordPart::Length(_) | WordPart::ArrayLength(_) => true,
            WordPart::ArrayAccess(reference) => {
                !reference_has_array_selector(reference, self.source)
                    && self.reference_is_field_safe(reference, span, context)
            }
            WordPart::Substring { reference, .. } => {
                self.reference_is_field_safe(reference, span, context)
            }
            WordPart::Transformation {
                reference,
                operator,
            } => self.transformation_is_field_safe(reference, *operator, span, context),
            WordPart::IndirectExpansion { name, .. } => {
                self.indirect_name_is_field_safe(name, span, context)
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch(_)
            | WordPart::ProcessSubstitution { .. } => false,
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => self.parameter_expansion_is_field_safe(reference, operator, span, context),
        }
    }

    fn literal_part_is_field_safe(
        &self,
        part: &WordPart,
        span: Span,
        context: ExpansionContext,
    ) -> bool {
        let word = Word {
            parts: vec![WordPartNode::new(part.clone(), span)],
            span,
        };
        classify_contextual_operand(&word, self.source, context).is_fixed_literal()
            && static_word_text(&word, self.source).is_some_and(|text| literal_is_field_safe(&text))
    }

    fn name_is_field_safe(&mut self, name: &Name, at: Span, context: ExpansionContext) -> bool {
        if self.maybe_uninitialized_refs.contains(&SpanKey::new(at)) {
            return false;
        }
        if safe_special_parameter(name) {
            return true;
        }

        let Some(binding) = self.semantic.visible_binding(name, at) else {
            return false;
        };
        self.binding_is_field_safe(binding.id, context)
    }

    fn binding_is_field_safe(&mut self, binding_id: BindingId, context: ExpansionContext) -> bool {
        let binding = self.semantic.binding(binding_id);
        if binding.attributes.contains(BindingAttributes::INTEGER)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        let binding_key = SpanKey::new(binding.span);
        let query = SafeValueQuery::from_context(context);
        let key = (binding_key, query);
        if let Some(result) = self.memo.get(&key) {
            return *result;
        }
        if !self.visiting.insert(key) {
            return false;
        }

        let result = if let Some(word) = self.scalar_bindings.get(&binding_key).copied() {
            self.word_is_field_safe(word, context)
        } else {
            false
        };

        self.visiting.remove(&key);
        self.memo.insert(key, result);
        result
    }

    fn reference_is_field_safe(
        &mut self,
        reference: &VarRef,
        at: Span,
        context: ExpansionContext,
    ) -> bool {
        if reference_has_array_selector(reference, self.source) {
            return false;
        }
        self.name_is_field_safe(&reference.name, at, context)
    }

    fn indirect_name_is_field_safe(
        &mut self,
        name: &Name,
        at: Span,
        context: ExpansionContext,
    ) -> bool {
        if self.maybe_uninitialized_refs.contains(&SpanKey::new(at)) {
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
                .all(|target| self.binding_is_field_safe(target, context))
    }

    fn transformation_is_field_safe(
        &mut self,
        reference: &VarRef,
        operator: char,
        at: Span,
        context: ExpansionContext,
    ) -> bool {
        if reference_has_array_selector(reference, self.source) {
            return false;
        }

        match operator {
            'Q' | 'K' | 'k' => true,
            _ => self.reference_is_field_safe(reference, at, context),
        }
    }

    fn parameter_expansion_is_field_safe(
        &mut self,
        reference: &VarRef,
        operator: &ParameterOp,
        at: Span,
        context: ExpansionContext,
    ) -> bool {
        if reference_has_array_selector(reference, self.source) {
            return false;
        }

        match operator {
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => self.reference_is_field_safe(reference, at, context),
            _ => false,
        }
    }

    fn word_is_field_safe(&mut self, word: &Word, context: ExpansionContext) -> bool {
        let analysis = analyze_word(word, self.source);
        if analysis.array_valued || analysis.hazards.command_or_process_substitution {
            return false;
        }

        word.parts_with_spans()
            .all(|(part, span)| self.part_is_field_safe(part, span, context))
    }
}

fn matches_scalar_expansion_part(part: &WordPart, source: &str) -> bool {
    match part {
        WordPart::Literal(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::DoubleQuoted { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ProcessSubstitution { .. } => false,
        WordPart::Variable(_)
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::Substring { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch(_)
        | WordPart::Transformation { .. } => true,
        WordPart::ArrayAccess(reference) => !reference_has_array_selector(reference, source),
        WordPart::ArrayIndices(_) | WordPart::ArraySlice { .. } => false,
    }
}

fn literal_is_field_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '*' | '?' | '['))
}

fn safe_special_parameter(name: &Name) -> bool {
    matches!(name.as_str(), "@" | "#" | "?" | "$" | "!" | "-")
}

fn reference_has_array_selector(reference: &VarRef, _source: &str) -> bool {
    matches!(
        reference.subscript.as_ref().map(|subscript| subscript.kind),
        Some(shuck_ast::SubscriptKind::Selector(
            SubscriptSelector::At | SubscriptSelector::Star
        ))
    )
}

fn report_word_expansions(
    checker: &mut Checker,
    indexer: &shuck_indexer::Indexer,
    safe_values: &mut SafeValueIndex<'_>,
    word: &Word,
    context: ExpansionContext,
    source: &str,
) {
    let classification = classify_word(word, source);
    if !classification.has_scalar_expansion() {
        return;
    }

    for (part, part_span) in word.parts_with_spans() {
        if !matches_scalar_expansion_part(part, source) {
            continue;
        }
        if span::is_quoted_span(indexer, part_span) {
            continue;
        }
        if safe_values.part_is_field_safe(part, part_span, context) {
            continue;
        }

        checker.report_dedup(UnquotedExpansion, part_span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_scalar_expansion_parts_instead_of_whole_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${name}suffix ${arr[0]} ${arr[@]}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}", "${arr[0]}"]
        );
    }

    #[test]
    fn descends_into_nested_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$(echo $name)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn ignores_expansions_inside_quoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
exec dbus-send --bus=\"unix:path=$XDG_RUNTIME_DIR/bus\" / org.freedesktop.DBus.Peer.Ping
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_for_lists_but_still_reports_redirect_targets() {
        let source = "\
#!/bin/bash
for item in $first \"$second\"; do :; done
cat <<< $here >$out
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$out"]
        );
    }

    #[test]
    fn skips_assignment_values_and_descriptor_dup_targets() {
        let source = "\
#!/bin/bash
value=$name
printf '%s\\n' ok >&$fd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unquoted_spans_inside_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'prefix:'$name':suffix'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn skips_safe_special_parameters() {
        let source = "\
#!/bin/bash
printf '%s\\n' $? $# $$ $! $- $0 $1 $* $@
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$1", "$*"]
        );
    }

    #[test]
    fn skips_bindings_with_safe_visible_values() {
        let source = "\
#!/bin/bash
n=42
s=abc
glob='*'
split='1 2'
copy=\"$n\"
alias=$s
printf '%s\\n' $n $s $glob $split $copy $alias
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$glob", "$split"]
        );
    }

    #[test]
    fn skips_bindings_derived_from_arithmetic_values() {
        let source = "\
#!/bin/bash
x=$((1 + 2))
y=\"$x\"
z=${x}
printf '%s\\n' $x $y $z
if [ $x -eq 0 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_safe_indirect_and_transformed_bindings() {
        let source = "\
#!/bin/bash
base=abc
name=base
upper=${base^^}
value='a b*'
quoted=${value@Q}
printf '%s\\n' ${!name} $upper $quoted
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn indirect_cycles_and_multi_field_targets_stay_unsafe() {
        let source = "\
#!/bin/bash
split='1 2'
name=split
a=$b
b=$a
printf '%s\\n' ${!name} $a
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!name}", "$a"]
        );
    }
}
