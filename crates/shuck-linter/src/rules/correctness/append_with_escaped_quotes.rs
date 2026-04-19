use crate::rules::correctness::shell_quoting_reuse::analyze_shell_quoting_reuse;
use crate::{Checker, Rule, Violation};

pub struct AppendWithEscapedQuotes;

impl Violation for AppendWithEscapedQuotes {
    fn rule() -> Rule {
        Rule::AppendWithEscapedQuotes
    }

    fn message(&self) -> String {
        "quotes or backslashes stored in this value will stay literal on reuse".to_owned()
    }
}

pub fn append_with_escaped_quotes(checker: &mut Checker) {
    checker.report_all_dedup(
        analyze_shell_quoting_reuse(checker).assignment_spans,
        || AppendWithEscapedQuotes,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_assignment_sites_for_shell_encoded_values() {
        let source = "\
#!/bin/sh
args='--name \"hello world\"'\n\
copy=$args\n\
printf '%s\\n' $copy\n\
CFLAGS=\" -DDIR=\\\"$PREFIX/share/\\\"\"\n\
$CC $CFLAGS -c test.c -o test.o\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        let spans = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(spans, vec!["'--name \"hello world\"'", " -DDIR=\\\"",]);
    }

    #[test]
    fn ignores_safe_assignments_without_unsafe_reuse() {
        let source = "\
#!/bin/sh
args='--name \"hello world\"'\n\
printf '%s\\n' \"$args\"\n\
toolchain=\"--llvm-targets-to-build='X86;ARM;AArch64'\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_eval_only_reuse_sites() {
        let source = "\
#!/bin/bash
cmd='printf \"hello world\"'\n\
eval $cmd\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_assignments_reused_by_export_and_here_strings() {
        let source = "\
#!/bin/bash
args='--name \"hello world\"'\n\
export args\n\
cat <<< $args\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "'--name \"hello world\"'"
        );
    }

    #[test]
    fn anchors_multiline_literal_runs_before_the_next_expansion() {
        let source = "\
#!/bin/sh
BUILDARCH=x86_64\n\
_USE_INTERNAL_LIBS=1\n\
PKG=/tmp/pkg\n\
MAKE_ARGS=\"ARCH=$BUILDARCH \\\n\
USE_INTERNAL_LIBS=${_USE_INTERNAL_LIBS} \\\n\
USE_CODEC_VORBIS=1 \\\n\
USE_CODEC_OPUS=1 \\\n\
USE_FREETYPE=1 \\\n\
COPYDIR=\\\"$PKG/usr/share/games/rtcw\\\"\"\n\
make $MAKE_ARGS release\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            " \\\nUSE_CODEC_VORBIS=1 \\\nUSE_CODEC_OPUS=1 \\\nUSE_FREETYPE=1 \\\nCOPYDIR=\\\""
        );
    }

    #[test]
    fn keeps_literal_prefixes_that_end_right_before_an_expansion() {
        let source = "\
#!/bin/sh
category=dev-lang\n\
pkg=ocaml\n\
slot=0\n\
tobuildstr=\"\\\">=$category/$pkg:$slot\\\" $tobuildstr\"\n\
echo Building $tobuildstr\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\\\">=");
    }

    #[test]
    fn trims_single_quoted_prefixes_before_mixed_expansion_fragments() {
        let source = "\
#!/bin/sh
is_outbounds='outbounds:[{tag:'\\\"$is_config_name\\\"',protocol:'\\\"$is_protocol\\\"'}'\n\
printf '%s\\n' $is_outbounds\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\\\"");
    }

    #[test]
    fn keeps_single_quoted_shell_fragments_intact_when_dollar_text_is_literal() {
        let source = "\
#!/bin/sh
as_echo_body='eval expr \"X$1\" : \"X\\\\(.*\\\\)\"'\n\
sh -c $as_echo_body\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "'eval expr \"X$1\" : \"X\\\\(.*\\\\)\"'"
        );
    }

    #[test]
    fn keeps_double_quoted_prefixes_that_store_single_quoted_arguments() {
        let source = "\
#!/bin/sh
HW=\"-DDEVICES='$DEVICES'\"\n\
make $HW\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-DDEVICES='");
    }

    #[test]
    fn keeps_double_quoted_prefixes_that_wrap_project_closure_arguments() {
        let source = "\
#!/bin/sh
OPTS=\"${OPTS} -c '${CONF_FILE}'\"\n\
exec udevmon ${OPTS}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), " -c '");
    }

    #[test]
    fn reports_shell_encoded_values_reexported_through_repeated_export_targets() {
        let source = "\
#!/bin/sh
mode=PATCH\n\
sig_version=v1\n\
sig_key_id=v2\n\
sig_alg=v3\n\
sig_headers=v4\n\
signed=\"Authorization: Signature version=\\\"$sig_version\\\",keyId=\\\"$sig_key_id\\\",algorithm=\\\"$sig_alg\\\",headers=\\\"$sig_headers\\\"\"\n\
if [ \"$mode\" = GET ]; then\n\
  export _H2=\"$signed\"\n\
  printf '%s\\n' ok\n\
elif [ \"$mode\" = PATCH ]; then\n\
  # shellcheck disable=SC2090\n\
  export _H2=y\n\
  printf '%s\\n' ok\n\
fi\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "Authorization: Signature version=\\\""
        );
    }

    #[test]
    fn reports_single_quoted_backslash_newline_values_reused_unquoted() {
        let source = "\
#!/bin/sh
args='foo\\
bar'\n\
printf '%s\\n' $args\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo\\\nbar'");
    }
}
