use crate::facts::EscapeScanSourceKind;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};
use rustc_hash::FxHashSet;

pub struct EscapedUnderscore;

const FIX_TITLE: &str = "remove the needless backslash before the reported character";

impl Violation for EscapedUnderscore {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::EscapedUnderscore
    }

    fn message(&self) -> String {
        "a backslash before a regular character is unnecessary in a plain word".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some(FIX_TITLE.to_owned())
    }
}

pub fn escaped_underscore(checker: &mut Checker) {
    let escapes = checker
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
        .collect::<Vec<_>>();
    let non_fixable_spans = escapes
        .iter()
        .filter(|escape| escape.source_kind() == EscapeScanSourceKind::PatternCharClass)
        .map(|escape| (escape.span().start.offset, escape.span().end.offset))
        .collect::<FxHashSet<_>>();
    let diagnostics = escapes
        .into_iter()
        .map(|escape| {
            let backslash_span = shuck_ast::Span::from_positions(
                escape.span().start,
                escape.span().start.advanced_by("\\"),
            );
            let diagnostic = Diagnostic::new(EscapedUnderscore, escape.span());
            if non_fixable_spans.contains(&(escape.span().start.offset, escape.span().end.offset)) {
                diagnostic
            } else {
                diagnostic.with_fix(Fix::unsafe_edit(Edit::deletion(backslash_span)))
            }
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
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

    use super::FIX_TITLE;
    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{
        Applicability, Diagnostic, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff,
        lint_file_at_path_with_parse_result,
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
    fn attaches_unsafe_fix_metadata_for_plain_word_escapes() {
        let source = "#!/bin/bash\necho foo\\_bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EscapedUnderscore));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(diagnostics[0].fix_title.as_deref(), Some(FIX_TITLE));
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

    #[test]
    fn applies_unsafe_fix_to_plain_word_escapes() {
        let source = "\
#!/bin/bash
echo foo\\_bar
echo foo\\:bar
echo ${host}\\:443
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EscapedUnderscore),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
echo foo_bar
echo foo:bar
echo ${host}:443
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_plain_word_escapes_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
echo foo\\\\_bar
echo \"foo\\_bar\"
grep foo\\_bar file
srcnam=$(tr \\. _ <<<${PRGNAM#python3-*})
name=\"${name//[^a-zA-Z0-9_\\-]/}\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EscapedUnderscore),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_pattern_char_class_hyphen_escapes_unfixed() {
        let source = "\
#!/bin/sh
case \"$x\" in
  [a\\-z]) echo ok ;;
esac
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EscapedUnderscore).with_shell(ShellDialect::Sh),
            Applicability::Unsafe,
        );

        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].fix.is_none());
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
        assert!(result.fixed_diagnostics[0].fix.is_none());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S023.sh").as_path(),
            &LinterSettings::for_rule(Rule::EscapedUnderscore),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("S023_fix_S023.sh", result);
        Ok(())
    }
}
