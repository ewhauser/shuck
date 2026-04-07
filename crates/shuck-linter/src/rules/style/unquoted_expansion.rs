use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{AssignmentValue, DeclOperand, Name, Span, Word, WordPart};
use shuck_semantic::{BindingAttributes, BindingKind, SemanticModel};

use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
    word::classify_word,
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

                report_word_expansions(
                    checker,
                    &mut safe_values,
                    word,
                    PartSafetyMode::AllowSafeBindings,
                );
            });
        },
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartSafetyMode {
    AllowSafeBindings,
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

struct SafeValueIndex<'a> {
    semantic: &'a SemanticModel,
    source: &'a str,
    scalar_bindings: FxHashMap<SpanKey, &'a Word>,
    maybe_uninitialized_refs: FxHashSet<SpanKey>,
    memo: FxHashMap<SpanKey, bool>,
    visiting: FxHashSet<SpanKey>,
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

    fn part_is_field_safe(&mut self, part: &WordPart, span: Span) -> bool {
        match part {
            WordPart::Literal(text) => literal_is_field_safe(text.as_str(self.source, span)),
            WordPart::SingleQuoted { value, .. } => literal_is_field_safe(value.slice(self.source)),
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter()
                .all(|part| self.part_is_field_safe(&part.kind, part.span)),
            WordPart::Variable(name) => self.name_is_field_safe(name, span),
            WordPart::ArithmeticExpansion { .. } => true,
            WordPart::Length(_) | WordPart::ArrayLength(_) => true,
            WordPart::CommandSubstitution { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch(_)
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => false,
        }
    }

    fn name_is_field_safe(&mut self, name: &Name, at: Span) -> bool {
        if self.maybe_uninitialized_refs.contains(&SpanKey::new(at)) {
            return false;
        }
        if safe_special_parameter(name) {
            return true;
        }

        let Some(binding) = self.semantic.visible_binding(name, at) else {
            return false;
        };
        if binding.attributes.contains(BindingAttributes::INTEGER)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        let key = SpanKey::new(binding.span);
        if let Some(result) = self.memo.get(&key) {
            return *result;
        }
        if !self.visiting.insert(key) {
            return false;
        }

        let result = if let Some(word) = self.scalar_bindings.get(&key).copied() {
            self.word_is_field_safe(word)
        } else {
            false
        };

        self.visiting.remove(&key);
        self.memo.insert(key, result);
        result
    }

    fn word_is_field_safe(&mut self, word: &Word) -> bool {
        word.parts_with_spans()
            .all(|(part, span)| self.part_is_field_safe(part, span))
    }
}

fn matches_scalar_expansion_part(part: &WordPart) -> bool {
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
        WordPart::ArrayAccess(reference) => !reference.has_array_selector(),
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

fn report_word_expansions(
    checker: &mut Checker,
    safe_values: &mut SafeValueIndex<'_>,
    word: &Word,
    safety_mode: PartSafetyMode,
) {
    let classification = classify_word(word);
    if !classification.has_scalar_expansion() {
        return;
    }

    for (part, part_span) in word.parts_with_spans() {
        if !matches_scalar_expansion_part(part) {
            continue;
        }
        if matches!(safety_mode, PartSafetyMode::AllowSafeBindings)
            && safe_values.part_is_field_safe(part, part_span)
        {
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
}
