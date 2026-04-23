use crate::{Checker, Rule, ShellDialect, Violation};

pub struct CStyleForArithmeticInSh;

impl Violation for CStyleForArithmeticInSh {
    fn rule() -> Rule {
        Rule::CStyleForArithmeticInSh
    }

    fn message(&self) -> String {
        "arithmetic `++` and `--` operators are not portable in `sh` scripts".to_owned()
    }
}

pub fn c_style_for_arithmetic_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker.facts().arithmetic_update_operator_spans().to_vec();

    checker.report_all(spans, || CStyleForArithmeticInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_update_operators_inside_c_style_for() {
        let source = "#!/bin/sh\nfor ((++i; j < 3; k--)); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_standalone_arithmetic() {
        let source = "#!/bin/sh\n((++i))\n((j--))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_arithmetic_expansions() {
        let source = "#!/bin/sh\necho \"$((++i)) $((j--))\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_expanding_heredoc_bodies() {
        let source = "#!/bin/sh\ncat <<EOF\n$((++i))\n$(printf '%s' \"$((j--))\")\nEOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_heredoc_parameter_command_substitutions() {
        let source = "#!/bin/sh\ncat <<EOF\n${value:-$(printf '%s' \"$((i++))\")}\nEOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_assignment_target_subscripts() {
        let source = "#!/bin/sh\narr[i++]=x\narr[--j]=y\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_double_bracket_operands() {
        let source = "#!/bin/sh\n[[ \"$((i++))\" -gt 0 && \"$((j--))\" -lt 3 ]]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn ignores_associative_array_keys_that_look_like_updates() {
        let source = "#!/bin/sh\nlocal -A tools=([c++]=CXX)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_associative_assignment_target_subscripts_that_look_like_updates() {
        let source = "#!/bin/sh\nlocal -A tools[c++]=CXX\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_contextual_associative_assignment_target_subscripts_that_look_like_updates() {
        let source = "#!/bin/sh\ndeclare -A tools\ntools[c++]=CXX\ntools=([d--]=DASH)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_contextual_associative_reference_subscripts_that_look_like_updates() {
        let source = "#!/bin/sh\ndeclare -A tools\necho \"${tools[c++]}\"\n[[ ${tools[d--]} ]]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\nfor ((++i; j < 3; k--)); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
