use crate::{Checker, Rule, Violation};

pub struct SingleQuotedLiteral;

impl Violation for SingleQuotedLiteral {
    fn rule() -> Rule {
        Rule::SingleQuotedLiteral
    }

    fn message(&self) -> String {
        "shell expansion inside single quotes stays literal".to_owned()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ScanContext<'a> {
    command_name: Option<&'a str>,
    assignment_target: Option<&'a str>,
    variable_set_operand: bool,
}

pub fn single_quoted_literal(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter_map(|fragment| {
            let context = ScanContext {
                command_name: fragment.command_name(),
                assignment_target: fragment.assignment_target(),
                variable_set_operand: fragment.variable_set_operand(),
            };

            should_report_single_quoted_literal(fragment.span().slice(source), context)
                .then(|| fragment.span())
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(SingleQuotedLiteral, span);
    }
}

fn should_report_single_quoted_literal(text: &str, context: ScanContext<'_>) -> bool {
    if !contains_sc2016_trigger(text) || context.variable_set_operand {
        return false;
    }

    if context.command_name == Some("sed") {
        return !sed_text_is_exempt(text);
    }

    if context
        .assignment_target
        .is_some_and(assignment_target_is_exempt)
    {
        return false;
    }

    if context.command_name.is_some_and(command_name_is_exempt) {
        return false;
    }

    true
}

fn contains_sc2016_trigger(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] == b'$'
            && matches!(
                bytes[index + 1],
                b'{' | b'(' | b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
            )
        {
            return true;
        }

        if bytes[index] == b'`'
            && bytes.get(index + 1).is_some_and(|next| *next != b'`')
            && bytes[index + 2..].contains(&b'`')
        {
            return true;
        }

        index += 1;
    }

    false
}

fn sed_text_is_exempt(text: &str) -> bool {
    let bytes = text.as_bytes();

    for index in 0..bytes.len().saturating_sub(1) {
        if bytes[index] != b'$' {
            continue;
        }

        let next = bytes[index + 1];
        if !matches!(next, b'{' | b'd' | b'p' | b's' | b'a' | b'i' | b'c') {
            continue;
        }

        let following = bytes.get(index + 2).copied();
        if following.is_none_or(|byte| !byte.is_ascii_alphabetic()) {
            return true;
        }
    }

    false
}

fn assignment_target_is_exempt(target: &str) -> bool {
    matches!(target, "PS1" | "PS2" | "PS3" | "PS4" | "PROMPT_COMMAND")
}

fn command_name_is_exempt(command_name: &str) -> bool {
    matches!(
        command_name,
        "trap"
            | "sh"
            | "bash"
            | "ksh"
            | "zsh"
            | "ssh"
            | "eval"
            | "xprop"
            | "alias"
            | "sudo"
            | "doas"
            | "run0"
            | "docker"
            | "podman"
            | "oc"
            | "dpkg-query"
            | "jq"
            | "rename"
            | "rg"
            | "unset"
            | "git filter-branch"
            | "mumps -run %XCMD"
            | "mumps -run LOOP%XCMD"
    ) || command_name.ends_with("awk")
        || command_name.starts_with("perl")
}

#[cfg(test)]
mod tests {
    use super::{
        assignment_target_is_exempt, command_name_is_exempt, contains_sc2016_trigger,
        sed_text_is_exempt,
    };
    use crate::test::test_snippet;
    use crate::{Diagnostic, LinterSettings, Rule};

    fn c005(source: &str) -> usize {
        c005_diagnostics(source).len()
    }

    fn c005_diagnostics(source: &str) -> Vec<Diagnostic> {
        test_snippet(source, &LinterSettings::for_rule(Rule::SingleQuotedLiteral))
    }

    #[test]
    fn detects_sc2016_variable_like_sequences_and_backticks() {
        assert!(contains_sc2016_trigger("$HOME"));
        assert!(contains_sc2016_trigger("${name:-default}"));
        assert!(contains_sc2016_trigger("$(pwd)"));
        assert!(contains_sc2016_trigger("$1"));
        assert!(contains_sc2016_trigger("`pwd`"));
    }

    #[test]
    fn ignores_shellcheck_exempt_special_parameter_sequences() {
        for text in ["$$", "$?", "$#", "$@", "$*", "$!", "$-", "$", "hello world"] {
            assert!(!contains_sc2016_trigger(text), "{text}");
        }
    }

    #[test]
    fn recognizes_sed_exemptions() {
        assert!(sed_text_is_exempt("$p"));
        assert!(sed_text_is_exempt("${/lol/d}"));
        assert!(!sed_text_is_exempt("$pattern"));
    }

    #[test]
    fn recognizes_shellcheck_style_command_and_assignment_exemptions() {
        for command_name in [
            "awk",
            "gawk",
            "perl",
            "perl5.38",
            "trap",
            "alias",
            "jq",
            "git filter-branch",
        ] {
            assert!(command_name_is_exempt(command_name), "{command_name}");
        }

        for target in ["PS1", "PS2", "PS3", "PS4", "PROMPT_COMMAND"] {
            assert!(assignment_target_is_exempt(target), "{target}");
        }

        assert!(!command_name_is_exempt("echo"));
        assert!(!assignment_target_is_exempt("HOME"));
    }

    #[test]
    fn rule_detects_backticks_and_respects_exemptions() {
        assert_eq!(c005("echo '`pwd`'\n"), 1);
        assert_eq!(c005("echo '$@'\n"), 0);
        assert_eq!(c005("awk '{print $1}'\n"), 0);
        assert_eq!(c005("PS1='$PWD \\\\$ '\n"), 0);
        assert_eq!(c005("command jq '$__loc__'\n"), 0);
        assert_eq!(c005("sed -n '$p'\n"), 0);
        assert_eq!(c005("sed -n '$pattern'\n"), 1);
    }

    #[test]
    fn corpus_regression_teamcity_awk_is_exempt() {
        assert_eq!(c005("awk '{print $5}' || :\n"), 0);
    }

    #[test]
    fn corpus_regression_alias_wrapper_is_exempt() {
        assert_eq!(c005("alias hosts='sudo $EDITOR /etc/hosts'\n"), 0);
    }

    #[test]
    fn corpus_regression_special_parameters_are_exempt() {
        assert_eq!(c005("SHOBJ_LDFLAGS='-shared -Wl,-h,$@'\n"), 0);
        assert_eq!(c005("SHOBJ_LDFLAGS='-G -dy -z text -i -h $@'\n"), 0);
    }

    #[test]
    fn corpus_regression_backticks_are_reported() {
        let diagnostics = c005_diagnostics("SHOBJ_ARCHFLAGS='-arch_only `/usr/bin/arch`'\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 17);
    }

    #[test]
    fn corpus_regression_openvpn_sample_anchors_on_opening_quote() {
        let diagnostics = c005_diagnostics(
            "if ! grep -q sbin <<< \"$PATH\"; then\n\techo '$PATH does not include sbin. Try using \"su -\" instead of \"su\".'\nfi\n",
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 7);
    }

    #[test]
    fn diagnostic_span_covers_the_full_single_quoted_region() {
        let diagnostics = c005_diagnostics("echo '$HOME'\n");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.end.line, 1);
        assert_eq!(diagnostics[0].span.end.column, 13);
    }

    #[test]
    fn corpus_regression_omarchy_sample_anchors_on_opening_quote() {
        let diagnostics = c005_diagnostics(
            "  sed -i '/bindd = SUPER, RETURN, Terminal, exec, \\$terminal/ s|$| --working-directory=$(omarchy-cmd-terminal-cwd)|' ~/.config/hypr/bindings.conf\n",
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 10);
    }

    #[test]
    fn variable_set_operand_helper_does_not_panic_on_incomplete_operands() {
        assert_eq!(c005("test -v\n"), 0);
        assert_eq!(c005("test -v name\n"), 0);
    }

    #[test]
    fn reports_single_quoted_literals_inside_case_patterns() {
        assert_eq!(c005("case $x in '$HOME') : ;; esac\n"), 1);
    }

    #[test]
    fn reports_single_quoted_literals_inside_parameter_patterns() {
        assert_eq!(c005("echo ${value#'$HOME'}\n"), 1);
    }

    #[test]
    fn reports_single_quoted_literals_inside_keyed_array_subscripts() {
        assert_eq!(c005("declare -A map=(['$HOME']=1)\n"), 1);
    }

    #[test]
    fn reports_single_quoted_literals_inside_heredoc_bodies() {
        assert_eq!(
            c005("cat <<EOF\n'$HOME should expand but does not'\nEOF\n",),
            1
        );
    }

    #[test]
    fn reports_multiple_single_quoted_literals_inside_heredoc_bodies() {
        let source = "cat <<EOF\n'$HOME' and '$(pwd)'\nEOF\n";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'$HOME'", "'$(pwd)'"]
        );
    }

    #[test]
    fn reports_single_quoted_literals_inside_tab_stripped_heredoc_bodies() {
        assert_eq!(c005("cat <<-EOF\n\t'$HOME'\nEOF\n"), 1);
    }

    #[test]
    fn ignores_unmatched_single_quotes_inside_heredoc_bodies() {
        assert_eq!(c005("cat <<EOF\n'$HOME\nEOF\n"), 0);
    }

    #[test]
    fn ignores_escaped_single_quotes_inside_heredoc_bodies() {
        assert_eq!(c005("cat <<EOF\n\\'$HOME\\'\nEOF\n"), 0);
    }

    #[test]
    fn ignores_single_quotes_paired_across_heredoc_newlines() {
        assert_eq!(c005("cat <<EOF\n'$HOME\nstill here'\nEOF\n"), 0);
    }

    #[test]
    fn ignores_single_quoted_sequences_inside_quoted_heredoc_bodies() {
        assert_eq!(c005("cat <<'EOF'\n'$HOME'\nEOF\n"), 0);
    }
}
