use crate::{Checker, Rule, ShellDialect, Violation};

pub struct AnsiCQuoting;

impl Violation for AnsiCQuoting {
    fn rule() -> Rule {
        Rule::AnsiCQuoting
    }

    fn message(&self) -> String {
        "ANSI-C quoting is not portable in `sh`".to_owned()
    }
}

pub fn ansi_c_quoting(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter(|fragment| fragment.dollar_quoted())
        .filter(|fragment| is_well_formed_ansi_c_quote(fragment.span(), checker.source()))
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AnsiCQuoting);
}

fn is_well_formed_ansi_c_quote(span: shuck_ast::Span, source: &str) -> bool {
    let text = span.slice(source);
    text.starts_with("$'") && text.len() >= 3 && text.ends_with('\'')
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_each_ansi_c_quoted_fragment() {
        let source = "printf '%s\\n' $'line\\n' \"$'inner'\" plain='ok' $'tab\\t'\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AnsiCQuoting).with_shell(ShellDialect::Sh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$'line\\n'", "$'inner'", "$'tab\\t'"]
        );
    }

    #[test]
    fn ignores_plain_single_quotes_and_bash() {
        let source = "printf '%s\\n' 'plain' $'bash-only'\n";

        let sh_diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AnsiCQuoting).with_shell(ShellDialect::Sh),
        );
        let bash_diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AnsiCQuoting).with_shell(ShellDialect::Bash),
        );

        assert_eq!(
            sh_diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$'bash-only'"]
        );
        assert!(bash_diagnostics.is_empty());
    }

    #[test]
    fn ignores_trailing_dollar_before_quote_inside_double_quoted_strings() {
        let source = "\
#!/bin/sh
_socat_cert_cmd=\"echo '${_cmdpfx}show ssl cert' | socat '${_statssock}' - | grep -q '^${_pem}$'\"
_socat_crtlist_show_cmd=\"echo '${_cmdpfx}show ssl crt-list' | socat '${_statssock}' - | grep -q '^${Le_Deploy_haproxy_pem_path}$'\"
_socat_cert_commit_cmd=\"echo '${_cmdpfx}commit ssl cert ${_pem}' | socat '${_statssock}' - | grep -q '^Success!$'\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AnsiCQuoting).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_ansi_c_quoting_in_replacement_patterns_for_indirect_expansions() {
        let source = "\
#!/bin/sh
show_pkg_var \"$var\" \"${!var//$'\\n'/' '}\"\n\
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AnsiCQuoting).with_shell(ShellDialect::Sh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$'\\n'"]
        );
    }
}
