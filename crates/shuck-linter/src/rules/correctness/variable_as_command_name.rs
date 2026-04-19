use crate::rules::correctness::shell_quoting_reuse::analyze_shell_quoting_reuse;
use crate::{Checker, Rule, Violation};

pub struct VariableAsCommandName;

impl Violation for VariableAsCommandName {
    fn rule() -> Rule {
        Rule::VariableAsCommandName
    }

    fn message(&self) -> String {
        "unquoted expansion will not honor quotes or escapes stored in this variable".to_owned()
    }
}

pub fn variable_as_command_name(checker: &mut Checker) {
    checker.report_all_dedup(analyze_shell_quoting_reuse(checker).use_spans, || {
        VariableAsCommandName
    });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_argument_uses_of_shell_encoded_values() {
        let source = "\
#!/bin/sh
args='--name \"hello world\"'
printf '%s\n' $args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$args");
    }

    #[test]
    fn reports_command_names_here_strings_and_composite_words() {
        let source = "\
#!/bin/bash
cmd='printf \"hello world\"'
args='--name \"hello world\"'
$cmd
printf '%s\n' foo${args}bar
cat <<< $args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        let spans = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(spans, vec!["$cmd", "${args}", "$args"]);
    }

    #[test]
    fn propagates_shell_encoded_values_through_intermediate_scalars() {
        let source = "\
#!/bin/bash
toolchain=\"--llvm-targets-to-build='X86;ARM;AArch64'\"
build_flags=\"$toolchain --install-prefix=/tmp\"
printf '%s\n' $build_flags
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$build_flags");
    }

    #[test]
    fn ignores_safe_quoted_and_eval_uses() {
        let source = "\
#!/bin/bash
cmd=printf
args='--name \"hello world\"'
$cmd '%s\n' ok
printf '%s\n' \"$args\"
cat <<< \"$args\"
eval printf '%s\n' $args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_exporting_shell_encoded_values() {
        let source = "\
#!/bin/sh
args='--name \"hello world\"'
export args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "args");
    }

    #[test]
    fn does_not_propagate_through_substring_transformations() {
        let source = "\
#!/bin/bash
style=\"\\`'\"
quote=\"${style:1:1}\"\n\
export quote
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_bracket_v_tests_but_not_other_variable_set_forms() {
        let source = "\
#!/bin/bash
args='--name \"hello world\"'
[ -v args ]
test -v args
[[ -v args ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "args");
    }

    #[test]
    fn reports_bracket_v_tests_when_quoted_value_was_set_in_an_earlier_function() {
        let source = "\
#!/bin/bash
normalize() {
  args='--name \"hello world\"'
}
[ -v args ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "args");
    }

    #[test]
    fn reports_bracket_v_tests_when_the_latest_binding_is_local() {
        let source = "\
#!/bin/bash
normalize() {
  local args='--name \"hello world\"'
}
[ -v args ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "args");
    }

    #[test]
    fn ignores_bracket_v_tests_when_the_first_quoted_assignment_comes_later() {
        let source = "\
#!/bin/bash
[ -v args ]
args='--name \"hello world\"'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_bracket_v_tests_after_a_later_safe_assignment() {
        let source = "\
#!/bin/bash
args='--name \"hello world\"'
args=safe
[ -v args ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_safe_bindings_do_not_hide_later_unsafe_bracket_v_bindings() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/bash
. ./helper.sh
args='--name \"hello world\"'
[ -v args ]
";

        fs::write(&main, source).unwrap();
        fs::write(&helper, "args=safe\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "args");
    }

    #[test]
    fn reports_unquoted_reuse_of_single_quoted_backslash_newline_values() {
        let source = "\
#!/bin/sh
args='foo\\
bar'\n\
printf '%s\\n' $args\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$args");
    }
}
