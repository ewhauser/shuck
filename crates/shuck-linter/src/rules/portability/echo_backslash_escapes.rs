use crate::{Checker, Rule, ShellDialect, Violation};

pub struct EchoBackslashEscapes;

impl Violation for EchoBackslashEscapes {
    fn rule() -> Rule {
        Rule::EchoBackslashEscapes
    }

    fn message(&self) -> String {
        "use `printf` instead of relying on backslash escapes in `echo`".to_owned()
    }
}

pub fn echo_backslash_escapes(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Ksh
    ) {
        return;
    }

    checker.report_all_dedup(
        checker.facts().echo_backslash_escape_word_spans().to_vec(),
        || EchoBackslashEscapes,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_literal_echo_operands_with_portability_sensitive_backslash_escapes() {
        let source = r#"#!/bin/sh
echo \n
echo "\\n"
echo '\\n'
echo foo\nbar
echo "foo\nbar"
echo 'foo\\nbar'
echo \x41
echo \077
echo "See \`pyenv help <command>' for information on a specific command."
echo 'It'\''s just quote plumbing'
echo "  echo You may need to get \\\`FluidR3_GM.sf2\\' from somewhere"
echo "sed -e 's|^\\(CertStore=\\).*|\\1X|g'"
echo "prefix $VAR \\0 suffix"
echo -DLATEX=\\"$(which latex)\\"
echo "  .TargetPath = \"\\\\host.lan\\Data\""
echo -e "\n"
echo -n -e "\n"
echo -n "$flag" -e \x41
echo \c
echo \u1234
echo -n "\"${shortname}\""
echo "include \"$TERMUX_PREFIX/share/nano/*nanorc\""
echo "  .TargetPath = \"\\host.lan\\Data\""
command echo \n
builtin echo \n
printf '%s\n' \n
"#;
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoBackslashEscapes),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\\n",
                "\"\\\\n\"",
                "'\\\\n'",
                "foo\\nbar",
                "\"foo\\nbar\"",
                "'foo\\\\nbar'",
                "\\x41",
                "\\077",
                "\"  echo You may need to get \\\\\\`FluidR3_GM.sf2\\\\' from somewhere\"",
                "\"sed -e 's|^\\\\(CertStore=\\\\).*|\\\\1X|g'\"",
                "\"prefix $VAR \\\\0 suffix\"",
                "-DLATEX=\\\\\"$(which latex)\\\\\"",
                "\"  .TargetPath = \\\"\\\\\\\\host.lan\\\\Data\\\"\"",
                "\\x41",
            ]
        );
    }

    #[test]
    fn ignores_shells_outside_the_rule_target_set() {
        let source = "\
#!/bin/dash
echo \\n
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoBackslashEscapes),
        );

        assert!(diagnostics.is_empty());
    }
}
