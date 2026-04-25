use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SingleQuotedLiteral;

impl Violation for SingleQuotedLiteral {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::SingleQuotedLiteral
    }

    fn message(&self) -> String {
        "shell expansion inside single quotes stays literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the fragment with double quotes".to_owned())
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
    let diagnostics = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter_map(|fragment| {
            if fragment.dollar_quoted() {
                return None;
            }

            let context = ScanContext {
                command_name: fragment.command_name(),
                assignment_target: fragment.assignment_target(),
                variable_set_operand: fragment.variable_set_operand(),
            };

            should_report_single_quoted_literal(fragment.span().slice(source), context).then(|| {
                let diagnostic =
                    crate::Diagnostic::new(SingleQuotedLiteral, fragment.diagnostic_span());
                match single_quoted_literal_fix(fragment.span(), fragment.span().slice(source)) {
                    Some(fix) => diagnostic.with_fix(fix),
                    None => diagnostic,
                }
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn single_quoted_literal_fix(span: shuck_ast::Span, text: &str) -> Option<Fix> {
    quoted_fragment_to_double_quotes(text)
        .map(|replacement| Fix::unsafe_edit(Edit::replacement(replacement, span)))
}

fn quoted_fragment_to_double_quotes(text: &str) -> Option<String> {
    if !(text.starts_with('\'') && text.ends_with('\'')) {
        return None;
    }

    let body = &text[1..text.len() - 1];
    let mut replacement = String::with_capacity(body.len() + 2);
    replacement.push('"');
    for ch in body.chars() {
        match ch {
            '"' | '\\' => {
                replacement.push('\\');
                replacement.push(ch);
            }
            _ => replacement.push(ch),
        }
    }
    replacement.push('"');
    Some(replacement)
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
    use std::path::Path;

    use super::{
        assignment_target_is_exempt, command_name_is_exempt, contains_sc2016_trigger,
        quoted_fragment_to_double_quotes, sed_text_is_exempt,
    };
    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, Diagnostic, LinterSettings, Rule, assert_diagnostics_diff};

    fn c005(source: &str) -> usize {
        c005_diagnostics(source).len()
    }

    fn c005_diagnostics(source: &str) -> Vec<Diagnostic> {
        test_snippet(source, &LinterSettings::for_rule(Rule::SingleQuotedLiteral))
    }

    #[test]
    fn rewrites_plain_single_quoted_fragments_as_double_quoted_fragments() {
        assert_eq!(
            quoted_fragment_to_double_quotes("'$HOME'"),
            Some("\"$HOME\"".to_owned())
        );
        assert_eq!(
            quoted_fragment_to_double_quotes("'\"$HOME\" and \\\\path'"),
            Some("\"\\\"$HOME\\\" and \\\\\\\\path\"".to_owned())
        );
        assert_eq!(quoted_fragment_to_double_quotes("$'$HOME'"), None);
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
    fn diagnostic_span_covers_the_full_single_quoted_region_and_attaches_fix_metadata() {
        let diagnostics = c005_diagnostics("echo '$HOME'\n");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.end.line, 1);
        assert_eq!(diagnostics[0].span.end.column, 13);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("rewrite the fragment with double quotes")
        );
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
    fn ignores_single_quoted_literals_split_by_double_quoted_expansions() {
        assert_eq!(c005("rx=${rx:-'prefix'\"$pkgname\"'suffix'}\n"), 0);
    }

    #[test]
    fn reports_single_quoted_literals_inside_keyed_array_subscripts() {
        assert_eq!(c005("declare -A map=(['$HOME']=1)\n"), 1);
    }

    #[test]
    fn applies_unsafe_fix_to_reported_single_quoted_fragments() {
        let source = "\
#!/bin/sh
echo '$HOME'
printf '%s\\n' '${value:-fallback}'
msg='$(pwd)'
echo '`pwd`'
echo '\"$HOME\" and \\\\path'
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SingleQuotedLiteral),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 5);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo \"$HOME\"
printf '%s\\n' \"${value:-fallback}\"
msg=\"$(pwd)\"
echo \"`pwd`\"
echo \"\\\"$HOME\\\" and \\\\\\\\path\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_ansi_c_single_quoted_fragments() {
        assert_eq!(c005("echo $'$HOME'\n"), 0);
        assert_eq!(
            c005("cmd --payload $'proxy_set_header X-Forwarded-Proto $scheme;'\n"),
            0
        );
    }

    #[test]
    fn leaves_ansi_c_single_quoted_fragments_unchanged_when_fixing() {
        let source = "echo $'$HOME'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SingleQuotedLiteral),
            Applicability::Unsafe,
        );

        assert!(result.diagnostics.is_empty());
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn corpus_regression_tmux_compgen_wordlist_is_reported() {
        let source = "if [[ $option_type ]]; then\n\
             _comp_cmd_tmux__value \"$subcommand\" \"$option_type\"\n\
             return\n\
         elif ((positional_start < 0)) && [[ $cur == -* ]]; then\n\
             _comp_compgen -- -W '\"${!options[@]}\"'\n\
             return\n\
         fi\n";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 5);
        assert_eq!(diagnostics[0].span.slice(source), "'\"${!options[@]}\"'");
    }

    #[test]
    fn multiline_arithmetic_for_headers_do_not_drop_earlier_function_body_fragments() {
        let source = "\
subcommand()
{
    if [[ $option_type ]]; then
        _value \"$subcommand\" \"$option_type\"
        return
    elif ((positional_start < 0)) && [[ $cur == -* ]]; then
        _comp_compgen -- -W '\"${!options[@]}\"'
        return
    fi

    local args_index=$positional_start
    local usage_args_index
    for ((\\
    usage_args_index = 0;  \\
    usage_args_index < ${#args[@]};  \\
    args_index++, usage_args_index++)); do
        :
    done
}
";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 7);
        assert_eq!(diagnostics[0].span.slice(source), "'\"${!options[@]}\"'");
    }

    #[test]
    fn multiline_sed_program_assignments_anchor_on_the_assignment_line() {
        let source = "\
lt_compile=`echo \"$ac_compile\" | $SED \\\n\
-e 's:.*FLAGS}\\{0,1\\} :&$lt_compiler_flag :; t' \\\n\
-e 's: [^ ]*conftest\\.: $lt_compiler_flag&:; t' \\\n\
-e 's:$: $lt_compiler_flag:'`\n";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(1, 42, 1, 86), (1, 90, 1, 134), (1, 138, 1, 163)]
        );
    }

    #[test]
    fn continued_command_arguments_stay_on_physical_lines() {
        let source = "\
sed -i -e 's/foo/$bar/' \\\n\
  -e 's/baz/$qux/' file\n";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'s/foo/$bar/'", "'s/baz/$qux/'"]
        );
    }

    #[test]
    fn dollar_paren_command_substitutions_stay_on_physical_lines() {
        let source = "\
x=$(printf %s \\\n\
'$HOME')\n";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            (
                diagnostics[0].span.start.line,
                diagnostics[0].span.start.column,
                diagnostics[0].span.end.line,
                diagnostics[0].span.end.column,
            ),
            (2, 1, 2, 8)
        );
    }

    #[test]
    fn backtick_sed_replacements_match_shellcheck_single_line_span() {
        let source = "\
relink_command=`$ECHO \"$compile_var$compile_command$compile_rpath\" | $SED 's%@OUTPUT@%\\$progdir/\\$file%g'`\n";
        let diagnostics = c005_diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            (
                diagnostics[0].span.start.line,
                diagnostics[0].span.start.column,
                diagnostics[0].span.end.line,
                diagnostics[0].span.end.column,
            ),
            (1, 75, 1, 104)
        );
    }

    #[test]
    fn ignores_single_quoted_sequences_inside_expanding_heredoc_bodies() {
        assert_eq!(
            c005("cat <<EOF\n'$HOME should expand but does not'\nEOF\n",),
            0
        );
    }

    #[test]
    fn ignores_multiple_single_quoted_sequences_inside_expanding_heredoc_bodies() {
        assert_eq!(c005("cat <<EOF\n'$HOME' and '$(pwd)'\nEOF\n"), 0);
    }

    #[test]
    fn ignores_single_quoted_sequences_inside_tab_stripped_heredoc_bodies() {
        assert_eq!(c005("cat <<-EOF\n\t'$HOME'\nEOF\n"), 0);
    }

    #[test]
    fn ignores_realistic_config_template_payloads_in_heredocs() {
        assert_eq!(
            c005("cat <<EOF > .cargo/config\ndirectory = '$(pwd)/vendor'\nEOF\n"),
            0
        );
    }

    #[test]
    fn ignores_single_quoted_here_strings_passed_to_shell_commands() {
        assert_eq!(
            c005(
                "bash --init-file \"${BASH_IT?}/bash_it.sh\" -i <<< '_bash-it-flash-term \"${#BASH_IT_THEME}\" \"${BASH_IT_THEME}\"'\n",
            ),
            0
        );
    }

    #[test]
    fn reports_single_quoted_plain_redirect_targets_for_exempt_commands() {
        assert_eq!(c005("bash > '$HOME'\n"), 1);
    }

    #[test]
    fn ignores_single_quoted_sequences_inside_quoted_heredoc_bodies() {
        assert_eq!(c005("cat <<'EOF'\n'$HOME'\nEOF\n"), 0);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C005.sh").as_path(),
            &LinterSettings::for_rule(Rule::SingleQuotedLiteral),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C005_fix_C005.sh", result);
        Ok(())
    }
}
