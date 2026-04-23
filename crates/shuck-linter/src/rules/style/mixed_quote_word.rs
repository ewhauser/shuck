use crate::{Checker, ExpansionContext, Rule, Violation, WordFactHostKind};

pub struct MixedQuoteWord;

impl Violation for MixedQuoteWord {
    fn rule() -> Rule {
        Rule::MixedQuoteWord
    }

    fn message(&self) -> String {
        "avoid mixing bare fragments between reopened double-quoted text".to_owned()
    }
}

pub fn mixed_quote_word(checker: &mut Checker) {
    let facts = checker.facts();
    let spans = [
        ExpansionContext::CommandName,
        ExpansionContext::CommandArgument,
        ExpansionContext::AssignmentValue,
        ExpansionContext::DeclarationAssignmentValue,
        ExpansionContext::StringTestOperand,
        ExpansionContext::CasePattern,
    ]
    .into_iter()
    .flat_map(|context| facts.expansion_word_facts(context))
    .chain(facts.case_subject_facts())
    .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
    .flat_map(|fact| {
        fact.unquoted_literal_between_double_quoted_segments_spans()
            .iter()
            .copied()
    })
    .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MixedQuoteWord);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bare_fragments_between_reopened_double_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"left \"middle\" right\" \"foo\"-\"bar\"
name=\"foo\"bar\"baz\"
declare local_name=\"foo\"bar\"baz\"
if [ \"foo\"bar\"baz\" = x ]; then :; fi
case \"foo\"bar\"baz\" in x) : ;; esac
case x in \"foo\"bar\"baz\") : ;; esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["middle", "-", "bar", "bar", "bar", "bar"]
        );
    }

    #[test]
    fn ignores_single_quotes_dynamic_middles_and_separator_literals() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'foo'bar'baz'
printf '%s\\n' \"foo\"${bar}\"baz\" \"foo\"$(printf '%s' x)\"baz\"
printf '%s\\n' \"$left\"-\"$right\" \"$left\".\"$right\" \"$left\"@\"$right\"
printf '%s\\n' \"foo\"/\"bar\" \"foo\"=\"bar\" \"foo\":\"bar\" \"foo\"?\"bar\"
if [[ x =~ \"foo\"bar\"baz\" ]]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_shellcheck_skipped_glob_query_and_append_assignment_fragments() {
        let source = "\
#!/bin/bash
echo_and_run \"find $PREFIX/lib -name \"librustc_*\" -xtype l\"
export CARGO_TARGET_\"${env_host}\"_RUSTFLAGS+=\" -C link-arg=$($CC -print-libgcc-file-name)\"
mkdir -p \"$TERMUX_GODIR\"/{bin,src,doc,lib,\"pkg/tool/$TERMUX_GOLANG_DIRNAME\",pkg/include}
curl \"${gotifywebhook}/message\"?token=\"${gotifytoken}\"
java_home=\"$(find \"$java_library_base/\"*1.\"$version\"* -type d -name 'Home*')\"
printf '%s\\n' \"foo\"user@host\"bar\" \"foo\"a+b\"bar\"
print \"\\
export EASYRSA_REQ_SERIAL=\\\"$EASYRSA_REQ_SERIAL\\\"\\
\" | sed -e s/a/b/
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert!(
            diagnostics.is_empty(),
            "{:?}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn reports_shellcheck_style_escaped_quote_separator_and_line_join_patterns() {
        let source = "\
#!/bin/bash
echo -e \"\"\\\"\"CDKey\"\\\"\"=\"\\\"\"${CODE}\"\\\"\"\"
escaped=x
sed -i /\"${escaped}_START\"/,/\"${escaped}_END\"/d file
x=\"$AWK '\"\\
\" {x};\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\\\"", "\\\"", "\\\"", "\\\"", "/,/", "\\\n"]
        );
    }

    #[test]
    fn reports_each_reopened_quote_line_join_in_one_word() {
        let source = "\
#!/bin/bash
lt_cv_sys_global_symbol_pipe=\"$AWK '\"\\
\"     {last_section=section};\"\\
\"     /^COFF SYMBOL TABLE/{next};\"\\
\"     ' prfx=^$ac_symprfx\"
nested=\"$AWK '\"\\
\"     {value=$(printf \"%s\" x);};\"\\
\"     /^COFF SYMBOL TABLE/{next};\"\\
\"     ' prfx=^$ac_symprfx\"
legacy=\"$AWK '\"\\
\"     {value=`printf \"%s\" x`;};\"\\
\"     /^COFF SYMBOL TABLE/{next};\"\\
\"     ' prfx=^$ac_symprfx\"
grouped=\"$AWK '\"\\
\"     {value=$( (printf x); printf \"%s\" y );};\"\\
\"     /^COFF SYMBOL TABLE/{next};\"\\
\"     ' prfx=^$ac_symprfx\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n",
                "\\\n", "\\\n"
            ]
        );
    }

    #[test]
    fn ignores_fully_quoted_nested_contexts_that_shellcheck_skips() {
        let source = "\
#!/bin/bash
options+=(U \"${option_msgs[\"U\"]}\")
result=\"$(regex \"#FFFFFF\" '^(#?([a-fA-F0-9]{6}|[a-fA-F0-9]{3}))$')\"
args+=(\"--latest\" \"$(is_boolean_yes \"$JENKINS_PLUGINS_LATEST\" && echo \"true\" || echo \"false\")\")
x=\"${VALUE:-\"false\"}\"
cmd=(dialog --menu \"Wireless ESSID: \\Zb$(iwgetid -r || echo \"none\")\\ZB\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert!(
            diagnostics.is_empty(),
            "{:?}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn still_reports_reopened_quotes_inside_nested_command_words() {
        let source = "\
#!/bin/bash
x=\"$(cmd \"a\".\"b\")\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["."]
        );
    }

    #[test]
    fn still_reports_reopened_quotes_after_complete_command_substitutions() {
        let source = "\
#!/bin/bash
echo \"$(cmd)\"x\"y\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["x"]
        );
    }

    #[test]
    fn reports_escaped_template_fragments_between_reopened_quotes() {
        let source = "\
#!/bin/bash
echo \"#!/bin/bash
this_dir=\\$(dirname \"\\$0\")
\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\\$0"]
        );
    }

    #[test]
    fn still_reports_reopened_quotes_after_quoted_literal_fragment_prefixes() {
        let source = "\
#!/bin/bash
printf '%s\\n' '$('\"foo\"parenmid\"baz\" '${'\"foo\"bracemid\"baz\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["parenmid", "bracemid"]
        );
    }

    #[test]
    fn still_reports_reopened_quotes_after_comment_text_inside_command_substitutions() {
        let source = "\
#!/bin/bash
echo $(echo x # $(
 )\"foo\"bar\"baz\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["bar"]
        );
    }

    #[test]
    fn still_reports_reopened_quotes_after_comment_text_inside_backticks() {
        let source = "\
#!/bin/bash
echo `echo x # $(
`\"foo\"bar\"baz\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["bar"]
        );
    }

    #[test]
    fn still_reports_reopened_quotes_after_hashes_inside_nested_double_quotes() {
        let source = "\
#!/bin/bash
echo $(printf \"%s\" \"x # $(printf y)\")\"foo\"bar\"baz\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["bar"]
        );
    }

    #[test]
    fn still_reports_reopened_quotes_after_comment_text_inside_process_substitutions() {
        let source = "\
#!/bin/bash
echo <(echo x # ${
 )\"foo\"bar\"baz\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["bar"]
        );
    }
}
