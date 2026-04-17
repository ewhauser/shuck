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
    use crate::{LinterSettings, Rule, ShellDialect};

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
    fn ignores_multiline_brace_expansions_with_line_continuations() {
        let source = "\
#!/bin/bash
cp -a $TARNAM/{AUTHOR.txt,\\
CONTRIBUTORS.md,COPYING.txt,ChangeLog.md,\\
OFL.txt,README.md,documentation} \\
   $PKG/usr/doc/$PRGNAM-$VERSION/
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_brace_expansions_with_escaped_spaces() {
        let source = "\
#!/bin/bash
echo {alpha,\\ beta}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_literal_braces_inside_trailing_comments() {
        let source = "\
#!/bin/bash
python3 -m installer --destdir \"$PKG\" dist/*.whl # > ${CWD}/INSTALL.OUTPUT 2>&1
n=\"$node_number\" # nodenumber_id e.g. network.city.${N}.0
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_xargs_inline_replace_options() {
        let source = "\
#!/bin/bash
xargs -I{} basename {}
xargs -0 -I {} mv {} {}.new
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_empty_brace_pairs_even_after_option_parsing_stops() {
        let source = "\
#!/bin/bash
xargs printf -I{}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_nonempty_xargs_like_literals_after_option_parsing_stops() {
        let source = "\
#!/bin/bash
xargs printf -I{x}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.column, 16);
        assert_eq!(diagnostics[1].span.start.column, 18);
    }

    #[test]
    fn ignores_empty_brace_pairs_in_general_literals() {
        let source = "\
#!/bin/bash
echo {}
echo address:{}
echo Block={}
jq . <<<{}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_nonempty_literal_braces_for_non_find_exec_forms() {
        let source = "\
#!/bin/bash
echo {value} +
myfind -exec echo {value} \\;
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
    fn ignores_even_backslash_runs_before_parameter_expansions() {
        let source = "\
#!/bin/bash
echo \\\\${name}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_template_placeholder_braces() {
        let source = "\
#!/bin/bash
go list -f {{.Dir}} \"$path\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 4);
        assert_eq!(diagnostics[0].span.start.column, 12);
        assert_eq!(diagnostics[1].span.start.column, 13);
        assert_eq!(diagnostics[2].span.start.column, 18);
        assert_eq!(diagnostics[3].span.start.column, 19);
    }

    #[test]
    fn reports_standalone_and_lone_literal_braces() {
        let source = "\
#!/bin/bash
nft add set inet shellcrash cn_ip6 { type ipv6_addr \\; flags interval \\; }
cut -d} -f1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 36);
        assert_eq!(diagnostics[1].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.column, 74);
        assert_eq!(diagnostics[2].span.start.line, 3);
        assert_eq!(diagnostics[2].span.start.column, 7);
    }

    #[test]
    fn ignores_escaped_dollar_before_real_parameter_expansion() {
        let source = "\
#!/bin/bash
crash_v_new=$(eval echo \\$${crashcore}_v)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
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

    #[test]
    fn ignores_parameter_expansions_in_assignment_only_commands() {
        let source = "\
#!/bin/sh
if [ ${COMPLIANCE_FINDINGS_FOUND} -eq 0 ]; then COMPLIANCE=\"${GREEN}V\"; else COMPLIANCE=\"${RED}X\"; fi
TARGET=${1:-/dev/stdin}
NTPSERVER=\"${curArg}\"
crypt=${crypt//\\\\/\\\\\\\\}
crypt=${crypt//\\//\\\\\\/}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralBraces).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_assignment_only_parameter_expansions_in_bash_mode() {
        let source = "\
#!/bin/bash
if [ -e /usr/bin/wx-config ]; then GUI=${GUI:-yes}; else GUI=${GUI:-no}; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralBraces).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_escaped_find_exec_placeholders() {
        let source = "\
#!/bin/bash
find $TERMUX_PKG_SRCDIR -mindepth 1 -maxdepth 1 -exec cp -a \\{\\} ./ \\;
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nested_parameter_expansion_braces() {
        let source = "\
#!/bin/bash
local args=${*:1:${#@}-1}
eval ac_env_${ac_var}_set=\\${${ac_var}+set}
if eval \\${ac_cv_prog_make_${ac_make}_set+:} false; then :
  :
fi
__rvm_find \"${rvm_bin_path:=$rvm_path/bin}\" -name \\*${ruby_at_gemset} -exec rm -rf '{}' \\;
exec {IPC_FIFO_FD}<>\"$IPC_FIFO\"
exec {IPC_FIFO_FD}>&-
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_simple_escaped_parameter_braces_even_with_later_expansions() {
        let source = "\
#!/bin/bash
./configure --libdir=\\${exec_prefix}/lib${LIBDIRSUFFIX}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));
        let positions = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
            .collect::<Vec<_>>();

        assert_eq!(positions, vec![(2, 24), (2, 36)]);
    }

    #[test]
    fn reports_simple_escaped_parameter_braces_even_after_nested_escape() {
        let source = "\
#!/bin/bash
echo \\${${name}}/\\${fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));
        let positions = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
            .collect::<Vec<_>>();

        assert_eq!(positions, vec![(2, 20), (2, 29)]);
    }

    #[test]
    fn reports_nft_literal_braces_with_dropped_raw_tokens() {
        let source = "\
#!/bin/bash
nft add rule inet shellcrash $1 tcp dport {\"$mix_port, $redir_port, $tproxy_port\"} return
nft add rule inet shellcrash $1 udp dport {443, 8443} return
nft add rule inet shellcrash mark_out meta mark $fwmark meta l4proto {tcp, udp} tproxy to :$tproxy_port
nft add chain inet fw4 forward { type filter hook forward priority filter \\; } 2>/dev/null
nft add chain inet fw4 input { type filter hook input priority filter \\; } 2>/dev/null
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));
        let positions = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
            .collect::<Vec<_>>();

        assert_eq!(
            positions,
            vec![
                (2, 43),
                (2, 82),
                (3, 43),
                (3, 53),
                (4, 70),
                (4, 79),
                (5, 32),
                (5, 78),
                (6, 30),
                (6, 74),
            ]
        );
    }
}
