use crate::facts::EscapeScanSourceKind;
use crate::{Checker, Rule, Violation};

pub struct EscapedUnderscore;

impl Violation for EscapedUnderscore {
    fn rule() -> Rule {
        Rule::EscapedUnderscore
    }

    fn message(&self) -> String {
        "a backslash before a regular character is unnecessary in a plain word".to_owned()
    }
}

pub fn escaped_underscore(checker: &mut Checker) {
    let spans = checker
        .facts()
        .escape_scan_matches()
        .iter()
        .copied()
        .filter(|escape| match escape.source_kind() {
            EscapeScanSourceKind::WordLiteralPart
            | EscapeScanSourceKind::RedirectLiteralSegment => {
                !escape.inside_single_quoted_fragment()
            }
            EscapeScanSourceKind::DynamicPathCommandName
            | EscapeScanSourceKind::PatternLiteral
            | EscapeScanSourceKind::PatternCharClass => true,
            EscapeScanSourceKind::ParameterPatternCharClass => false,
            EscapeScanSourceKind::SingleLiteralAssignmentWord
            | EscapeScanSourceKind::BacktickFragment => false,
        })
        .filter(|escape| {
            !(escape.is_grep_style_argument()
                || escape.is_tr_operand_argument()
                    && matches!(escape.escaped_byte(), b'.' | b'*' | b'?'))
        })
        .filter(|escape| match escape.source_kind() {
            EscapeScanSourceKind::PatternCharClass => escape.escaped_byte() == b'-',
            _ => is_regular_plain_word_escape_target(escape.escaped_byte()),
        })
        .map(|escape| escape.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EscapedUnderscore);
}

fn is_regular_plain_word_escape_target(byte: u8) -> bool {
    !matches!(
        byte,
        b' ' | b'\t'
            | b'\n'
            | b'.'
            | b'@'
            | b'#'
            | b'$'
            | b'`'
            | b'"'
            | b'\''
            | b'\\'
            | b'*'
            | b'?'
            | b'['
            | b']'
            | b'&'
            | b'|'
            | b';'
            | b'<'
            | b'>'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'~'
            | b'+'
            | b'!'
            | b'n'
            | b'r'
            | b't'
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect as ParseDialect};

    use crate::test::test_snippet;
    use crate::{
        Diagnostic, LinterSettings, Rule, ShellDialect, lint_file_at_path_with_parse_result,
    };

    fn test_posix_snippet_at_path(path: &Path, source: &str) -> Vec<Diagnostic> {
        let parse_result = Parser::with_dialect(source, ParseDialect::Posix).parse();
        let indexer = Indexer::new(source, &parse_result);
        let settings =
            LinterSettings::for_rule(Rule::EscapedUnderscore).with_shell(ShellDialect::Sh);
        lint_file_at_path_with_parse_result(
            &parse_result,
            source,
            &indexer,
            &settings,
            None,
            Some(path),
        )
    }

    #[test]
    fn reports_needless_backslashes_in_plain_words() {
        let source = "\
#!/bin/bash
echo foo\\_bar
echo foo\\+bar
echo foo\\\\_bar
echo \"foo\\_bar\"
echo prefix\"\\_\"suffix
EXPECTED_OUTPUT=$(printf \"\\033[0;35mMagenta-colored text\")
\\command --help
[[ x =~ foo\\_bar ]]
foo=${x#foo\\_bar}
echo foo\\:bar
        case x in *[!a-zA-Z0-9._/+\\-]*) continue ;; esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["", "", ""]
        );
    }

    #[test]
    fn reports_redirect_target_colon_escapes() {
        let source = "\
base64 -d ${vkb64} > ${rootfs}/var/db/xbps/keys/60\\:ae\\:0c\\:d6\\:f0\\:95\\:17\\:80\\:bc\\:93\\:46\\:7a\\:89\\:af\\:a3\\:2d.plist
";
        let diagnostics = test_posix_snippet_at_path(Path::new("/tmp/lxc-void"), source);

        assert_eq!(diagnostics.len(), 15);
    }

    #[test]
    fn ignores_nested_redirect_substitution_escapes() {
        let source = "\
read -r newest_tag < <(echo \"$newest_tags\" | grep -Po '(?<=^\"v)\\d+\\.\\d+\\.\\d+' | sort -Vr)
cat >\"$(printf \"_\\x09_character_tabulation.txt\")\" <<EOF
$(printf \"_\\x09_character_tabulation.txt\")
EOF
done < <(sed -e \"s/^$/\\xFF/g\" \"${BOOTSTRAP_TMPDIR}/packages.${architecture}\")
";
        let diagnostics = test_posix_snippet_at_path(Path::new("/tmp/nested-redirects"), source);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_dynamic_path_like_command_names() {
        let source = "\
#!/bin/bash
${bindir}/foo\\_bar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn ignores_escaped_at_signs() {
        let source = "\
#!/bin/bash
echo foo\\@bar
echo \"$rvm_path\"/gems/*\\@
cp --no-preserve=mode,ownership -rf \"${GOPATH}\"/pkg/mod/\"${go_module}\"\\@* ./\"${go_module##*/}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn keeps_grep_style_patterns_out_of_the_sc1001_family() {
        let source = "\
#!/bin/sh
grep foo\\_bar file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn keeps_tr_operands_out_of_the_sc1001_family() {
        let source = "\
#!/bin/bash
srcnam=$(tr \\. _ <<<${PRGNAM#python3-*})
src_ver=$(echo $VERSION | tr -d \\.)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_parameter_expansion_char_classes() {
        let source = "\
#!/bin/bash
name=\"${name//[^a-zA-Z0-9_\\-]/}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_literal_dot_escapes() {
        let source = "\
#!/bin/bash
echo foo\\.bar
echo gem5-$gem5_isa\\.$VARIANT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_escapes_adjacent_to_expansions() {
        let source = "\
#!/bin/bash
echo $VERSION\\_$(echo x)
echo ${host}\\:443
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn reports_escapes_outside_single_quoted_fragments() {
        let source = "\
#!/bin/bash
echo 'prefix'\\:suffix
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert_eq!(diagnostics.len(), 1);
    }
}
