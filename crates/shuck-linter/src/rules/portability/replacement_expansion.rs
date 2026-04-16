use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ReplacementExpansion;

impl Violation for ReplacementExpansion {
    fn rule() -> Rule {
        Rule::ReplacementExpansion
    }

    fn message(&self) -> String {
        "replacement expansion is not portable in `sh`".to_owned()
    }
}

pub fn replacement_expansion(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .replacement_expansion_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ReplacementExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_replacement_expansions_only() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${name//x/y}\" \"${name/x/y}\" \"${name/#x/y}\" \"${name/%x/y}\" \"${arr[0]//x/y}\" \"${arr[@]/x/y}\" \"${arr[*]//x}\" \"${name/${needle}/y}\" \"${name^^}\" \"${name:1}\" \"${!name//x/y}\" \"${name@Q}\"\n\
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${name//x/y}",
                "${name/x/y}",
                "${name/#x/y}",
                "${name/%x/y}",
                "${arr[0]//x/y}",
                "${arr[@]/x/y}",
                "${arr[*]//x}",
                "${name/${needle}/y}",
            ]
        );
    }

    #[test]
    fn anchors_on_replacement_expansions_inside_unquoted_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
Expected: '${commit//old/new}'
Escaped: '\\${commit//old/new}'
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${commit//old/new}"]
        );
    }

    #[test]
    fn anchors_on_complex_replacement_expansions_inside_unquoted_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
Expected: 'npx create-next-app@v${TERMUX_PKG_VERSION//\\~/-}'
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${TERMUX_PKG_VERSION//\\~/-}"]
        );
    }

    #[test]
    fn anchors_on_complex_replacement_expansions_inside_tab_stripped_heredocs() {
        let source = "#!/bin/sh\ncat <<-EOF\n\tExpected: 'npx create-next-app@v${TERMUX_PKG_VERSION//\\~/-}'\nEOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${TERMUX_PKG_VERSION//\\~/-}"]
        );
    }

    #[test]
    fn anchors_on_complex_replacement_expansions_in_termux_style_heredocs() {
        let source = "\
#!/bin/sh
cat > ./postinst <<-EOF
\t#!$TERMUX_PREFIX/bin/sh
\techo \"You must explicitly use 'npx create-next-app@v${TERMUX_PKG_VERSION//\\~/-}' to avoid the error of Missing field 'isPersistentCachingEnabled'\"
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${TERMUX_PKG_VERSION//\\~/-}"]
        );
    }

    #[test]
    fn ignores_literal_replacement_expansions_in_nested_heredoc_shell_contexts() {
        let source = "\
#!/bin/sh
cat <<EOF
$(printf '%s' '${commit//old/new}')
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn anchors_on_replacement_expansions_with_escaped_and_nested_operands() {
        let source = "\
#!/bin/sh
TERMUX_PKG_SRCURL=https://github.com/vercel/next.js/archive/refs/tags/v${TERMUX_PKG_VERSION//\\~/-}.tar.gz
printf '%s\n' \"${dest_dir//\\'/\\'\\\\\\'\\'}\"
local TERMUX_PKG_VERSION_EDITED=${TERMUX_PKG_VERSION//-/.}
local TERMUX_PKG_VERSION_EDITED=${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}}
echo \"Starting batch $BATCH at: ${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/}\"
run_depends=\"${run_depends/${i}/${dep}}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${TERMUX_PKG_VERSION//\\~/-}",
                "${dest_dir//\\'/\\'\\\\\\'\\'}",
                "${TERMUX_PKG_VERSION//-/.}",
                "${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}}",
                "${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/}",
                "${run_depends/${i}/${dep}}",
            ]
        );
    }

    #[test]
    fn anchors_on_replacement_expansions_with_complex_operator_bodies() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${dest_dir//\\'/\\'\\\\\\'\\'}\" \"${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}}\" \"${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/}\" \"${run_depends/${i}/${dep}}\"\n\
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${dest_dir//\\'/\\'\\\\\\'\\'}",
                "${TERMUX_PKG_VERSION_EDITED//${INCORRECT_SYMBOLS:0:1}${INCORRECT_SYMBOLS:1:1}/${INCORRECT_SYMBOLS:0:1}.${INCORRECT_SYMBOLS:1:1}}",
                "${GITHUB_GRAPHQL_QUERIES[$BATCH * $BATCH_SIZE]//\\\\/}",
                "${run_depends/${i}/${dep}}",
            ]
        );
    }

    #[test]
    fn ignores_replacement_expansion_in_bash() {
        let source = "printf '%s\n' \"${name//x/y}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
