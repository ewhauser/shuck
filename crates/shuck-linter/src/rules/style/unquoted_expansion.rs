use shuck_ast::{Word, WordPart};

use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
    safe_value::{SafeValueIndex, SafeValueQuery},
    word::classify_word,
};
use crate::{Checker, Rule, ShellDialect, Violation};

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
    let mut safe_values = SafeValueIndex::build(checker.semantic(), &checker.ast().body, source);

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let _command = visit.command;
            query::visit_expansion_words(visit, source, &mut |word, context| {
                if !should_check_context(context, checker.shell()) {
                    return;
                }

                report_word_expansions(checker, &mut safe_values, word, context, source);
            });
        },
    );
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
        | WordPart::PrefixMatch { .. }
        | WordPart::Transformation { .. } => true,
        WordPart::ArrayAccess(reference) => !reference.has_array_selector(),
        WordPart::ArrayIndices(_) | WordPart::ArraySlice { .. } => false,
    }
}

fn should_check_context(context: ExpansionContext, shell: ShellDialect) -> bool {
    match context {
        ExpansionContext::CommandName
        | ExpansionContext::CommandArgument
        | ExpansionContext::RedirectTarget(_) => true,
        ExpansionContext::DeclarationAssignmentValue => shell != ShellDialect::Bash,
        _ => false,
    }
}

fn command_name_has_literal_affixes(word: &Word) -> bool {
    word.parts.iter().any(|part| {
        matches!(
            part.kind,
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
        )
    })
}

fn report_word_expansions(
    checker: &mut Checker,
    safe_values: &mut SafeValueIndex<'_>,
    word: &Word,
    context: ExpansionContext,
    source: &str,
) {
    let classification = classify_word(word, source);
    if !classification.has_scalar_expansion() {
        return;
    }
    if context == ExpansionContext::CommandName && !command_name_has_literal_affixes(word) {
        return;
    }
    let query = SafeValueQuery::from_context(context)
        .expect("checked expansion context should map to a safe-value query");

    for (part, part_span) in word.parts_with_spans() {
        if !matches_scalar_expansion_part(part) {
            continue;
        }
        if safe_values.part_is_safe(part, part_span, query) {
            continue;
        }

        checker.report_dedup(UnquotedExpansion, part_span);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
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
    fn reports_only_unquoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix\"$HOME\"/$suffix
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix"]
        );
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
    fn reports_dynamic_command_names() {
        let source = "\
#!/bin/bash
$HOME/bin/tool $arg
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOME", "$arg"]
        );
    }

    #[test]
    fn skips_plain_expansion_command_names() {
        let source = "\
#!/bin/bash
$CC -c file.c
if $TERMUX_ON_DEVICE_BUILD; then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_escaped_backticks_inside_double_quoted_assignments() {
        let source = "\
#!/bin/bash
NVM_TEST_VERSION=v0.42
EXPECTED=\"Found '$(pwd)/.nvmrc' with version <${NVM_TEST_VERSION}>
N/A: version \\\"${NVM_TEST_VERSION}\\\" is not yet installed.

You need to run \\`nvm install ${NVM_TEST_VERSION}\\` to install and use it.
No NODE_VERSION provided; no .nvmrc file found\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn reports_decl_assignment_values_in_sh_mode() {
        let source = "\
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_BUILDER_DIR", "$HOME", "$PATH"]
        );
    }

    #[test]
    fn skips_decl_assignment_values_in_bash_mode() {
        let source = "\
#!/bin/bash
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.bash"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

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
