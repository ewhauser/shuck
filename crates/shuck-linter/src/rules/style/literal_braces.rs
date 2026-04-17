use crate::{Checker, Rule, Violation};

pub struct LiteralBraces;

impl Violation for LiteralBraces {
    fn rule() -> Rule {
        Rule::LiteralBraces
    }

    fn message(&self) -> String {
        "literal braces may be interpreted as brace syntax".to_owned()
    }
}

pub fn literal_braces(checker: &mut Checker) {
    checker.report_all_dedup(checker.facts().literal_brace_spans().to_vec(), || {
        LiteralBraces
    });
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_literal_unquoted_brace_pair_edges() {
        let source = "#!/bin/bash\necho HEAD@{1}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.column, 11);
        assert_eq!(diagnostics[1].span.start.column, 13);
    }

    #[test]
    fn ignores_quoted_and_expanding_braces() {
        let source = "#!/bin/bash\necho \"HEAD@{1}\" x{a,b}y\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_find_exec_placeholder_and_regex_quantifier() {
        let source = "\
#!/bin/bash
find . -exec echo {} \\;
if [[ \"$hash\" =~ ^[a-f0-9]{40}$ ]]; then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_wrapped_and_multiline_find_exec_placeholders() {
        let source = "\
#!/bin/bash
if ! find \"$output_dir\" -mindepth 1 -exec false {} + 2>/dev/null; then
  exit 1
fi
find \"$TERMUX_PREFIX\"/share/doc/\"$TERMUX_PKG_NAME\" \\
  -type f -execdir sed -i -e 's/\\r$//g' {} +
find . \\
  -exec sh -c 'is_empty \"$0\"' {} \\;
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_dynamic_brace_expansions() {
        let source = "\
#!/bin/bash
ln -sf \"${TERMUX_PREFIX}/lib/\"{libjanet.so.${TERMUX_PKG_VERSION},libjanet.so.${TERMUX_PKG_VERSION%.*}}
ln -sfr $TERMUX_PREFIX/lib/libaircrack-${m}{-$_LT_VER,}.so
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_xargs_inline_replace_options() {
        let source = "\
#!/bin/bash
xargs -I{} basename \"{}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_xargs_like_literals_after_option_parsing_stops() {
        let source = "\
#!/bin/bash
xargs printf -I{}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.column, 16);
        assert_eq!(diagnostics[1].span.start.column, 17);
    }

    #[test]
    fn reports_literal_braces_for_non_find_exec_forms() {
        let source = "\
#!/bin/bash
echo {} +
myfind -exec echo {} \\;
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 4);
    }

    #[test]
    fn reports_escaped_dollar_literal_braces() {
        let source = "\
#!/bin/bash
eval command sudo \\\"\\${sudo_args[@]}\\\"
echo [0-9a-f]{$HASHLEN}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 4);
    }

    #[test]
    fn ignores_plain_parameter_expansion_braces_next_to_brace_expansion() {
        let source = "\
#!/bin/bash
echo TERMUX_SUBPKG_INCLUDE=\\\"$(find ${_ADD_PREFIX}lib{,32} -name '*.a' -o -name '*.la' 2> /dev/null) $TERMUX_PKG_STATICSPLIT_EXTRA_PATTERNS\\\" > \"$_STATIC_SUBPACKAGE_FILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
