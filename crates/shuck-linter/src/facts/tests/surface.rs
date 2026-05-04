use super::*;
use crate::{
    FieldSplittingBehavior, GlobDotBehavior, GlobFailureBehavior, PathnameExpansionBehavior,
    PatternOperatorBehavior, PlainUnindexedArrayReferenceFact, SubscriptIndexBehavior,
};

#[test]
fn builds_surface_fragment_facts_and_static_utility_names() {
    let source = "\
#!/bin/bash
echo \"prefix `date` suffix\"
echo \"$[1 + 2]\"
arr[$10]=1
declare other[$10]=1
echo \"$(( x $1 y ))\"
PS4='$prompt'
command jq '$__loc__'
test -v '$name'
printf '%s\n' $'tab\t'
echo $\"Usage: $0 {start|stop}\"
printf '%s\n' \"${!name}\" \"${!arr[*]}\"
printf '%s\n' \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${#arr[0]}\" \"${#arr[@]}\" \"${arr[0]%x}\" \"${arr[0]:2}\" \"${arr[0]//x/y}\" \"${arr[0]:-fallback}\" \"${!arr[0]}\"
printf '%s\n' \"${name:2}\" \"${1:1}\" \"${name::2}\" \"${@:1}\" \"${*:1:2}\" \"${arr[@]:1}\" \"${arr[0]:1}\"
printf '%s\n' \"${@:-fallback}\" \"${name:-$10}\"
printf '%s\n' \"${name:-${@}}\"
printf '%s\n' \"${name^}\" \"${name^^pattern}\" \"${name,}\" \"${arr[0]^^}\" \"${arr[@],,}\" \"${!name^^}\" \"${name//x/y}\"
printf '%s\n' \"${name/a/b}\" \"${name//a}\" \"${arr[0]//a/b}\" \"${arr[@]/a/b}\" \"${arr[*]//a}\" \"${!name//a/b}\"
if [ \"$(dpkg-query -W -f '${db:Status-Status}\\n' package 2>/dev/null)\" != \"installed\" ]; then :; fi
cat <<EOF
Expected: '${expected_commit::7}'
#define LAST_COMMIT_POSITION \"2311 ${GN_COMMIT:0:12}\"
Replacement: '${name//before/after}'
EOF
printf '%s\\n' 123 | command kill -9
echo \"#!/bin/bash
if [[ \"$@\" =~ x ]]; then :; fi
\"
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .backtick_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`"]
        );
        assert_eq!(
            facts
                .legacy_arithmetic_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["$[1 + 2]"]
        );
        assert_eq!(
            facts
                .positional_parameter_fragments()
                .iter()
                .map(|fragment| {
                    (
                        fragment.span().slice(source),
                        fragment.kind(),
                        fragment.is_above_nine(),
                        fragment.is_guarded(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    "$10",
                    PositionalParameterFragmentKind::AboveNine,
                    true,
                    false
                ),
                (
                    "$10",
                    PositionalParameterFragmentKind::AboveNine,
                    true,
                    false
                ),
                (
                    "${@:1}",
                    PositionalParameterFragmentKind::General,
                    false,
                    false
                ),
                (
                    "${*:1:2}",
                    PositionalParameterFragmentKind::General,
                    false,
                    false
                ),
                (
                    "${@:-fallback}",
                    PositionalParameterFragmentKind::General,
                    false,
                    true
                ),
                (
                    "$10",
                    PositionalParameterFragmentKind::AboveNine,
                    true,
                    true
                ),
                (
                    "${@}",
                    PositionalParameterFragmentKind::General,
                    false,
                    true
                ),
            ]
        );
        assert_eq!(
            facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![""]
        );
        assert_eq!(
            facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| fragment.replacement_span().slice(source))
                .collect::<Vec<_>>(),
            vec!["\"#!/bin/bash\nif [[ \"$@\" =~ x ]]; then :; fi\n\""]
        );
        assert_eq!(
            facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| fragment.replacement().to_owned())
                .collect::<Vec<_>>(),
            vec!["\"#!/bin/bash\nif [[ ${@} =~ x ]]; then :; fi\n\"".to_owned()]
        );
        assert_eq!(
            facts
                .suspect_closing_quote_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![""]
        );
        assert_eq!(facts.positional_parameter_operator_spans().len(), 1);
        let operator_span = facts.positional_parameter_operator_spans()[0];
        assert_eq!(operator_span.start.line, 6);
        assert_eq!(operator_span.start.column, 7);
        assert_eq!(operator_span.end, operator_span.start);

        let single_quoted = facts
            .single_quoted_fragments()
            .iter()
            .map(|fragment| {
                (
                    fragment.span().slice(source).to_owned(),
                    fragment.dollar_quoted(),
                    fragment.command_name().map(str::to_owned),
                    fragment.assignment_target().map(str::to_owned),
                    fragment.variable_set_operand(),
                )
            })
            .collect::<Vec<_>>();
        assert!(single_quoted.iter().any(
            |(text, _, _, assignment_target, variable_set_operand)| {
                text == "'$prompt'"
                    && assignment_target.as_deref() == Some("PS4")
                    && !variable_set_operand
            }
        ));
        assert!(single_quoted.contains(&(
            "'$__loc__'".to_owned(),
            false,
            Some("jq".to_owned()),
            None,
            false,
        )));
        assert!(single_quoted.contains(&(
            "'$name'".to_owned(),
            false,
            Some("test".to_owned()),
            None,
            true,
        )));
        assert!(
            single_quoted
                .iter()
                .any(|(text, dollar_quoted, _, _, variable_set_operand)| {
                    text.starts_with("$'tab") && *dollar_quoted && !variable_set_operand
                })
        );
        assert_eq!(
            facts
                .dollar_double_quoted_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["$\"Usage: $0 {start|stop}\""]
        );
        assert_eq!(
            facts
                .indirect_expansion_fragments()
                .iter()
                .map(|fragment| (fragment.span().slice(source), fragment.array_keys()))
                .collect::<Vec<_>>(),
            vec![
                ("${!name}", false),
                ("${!arr[*]}", true),
                ("${!arr[0]}", false),
                ("${!name//a/b}", false),
            ]
        );
        assert_eq!(
            facts
                .indexed_array_reference_fragments()
                .iter()
                .map(|fragment| (fragment.span().slice(source), fragment.is_plain()))
                .collect::<Vec<_>>(),
            vec![
                ("${!arr[*]}", false),
                ("${arr[0]}", true),
                ("${arr[@]}", true),
                ("${arr[*]}", true),
                ("${#arr[0]}", false),
                ("${#arr[@]}", false),
                ("${arr[0]%x}", false),
                ("${arr[0]:2}", false),
                ("${arr[0]//x/y}", false),
                ("${arr[0]:-fallback}", false),
                ("${!arr[0]}", false),
                ("${arr[@]:1}", false),
                ("${arr[0]:1}", false),
                ("${arr[0]^^}", false),
                ("${arr[@],,}", false),
                ("${arr[0]//a/b}", false),
                ("${arr[@]/a/b}", false),
                ("${arr[*]//a}", false),
            ]
        );
        assert!(
            facts.zsh_parameter_index_flag_fragments().is_empty(),
            "did not expect quoted-target index facts in the baseline surface fixture"
        );
        assert_eq!(
            facts
                .substring_expansion_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${name:2}",
                "${1:1}",
                "${name::2}",
                "${@:1}",
                "${*:1:2}",
                "${expected_commit::7}",
                "${GN_COMMIT:0:12}",
            ]
        );
        assert_eq!(
            facts
                .case_modification_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${name^}",
                "${name^^pattern}",
                "${name,}",
                "${arr[0]^^}",
                "${arr[@],,}",
            ]
        );
        assert_eq!(
            facts
                .replacement_expansion_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${arr[0]//x/y}",
                "${name//x/y}",
                "${name/a/b}",
                "${name//a}",
                "${arr[0]//a/b}",
                "${arr[@]/a/b}",
                "${arr[*]//a}",
                "${name//before/after}",
            ]
        );

        let jq = facts
            .structural_commands()
            .find(|fact| fact.static_utility_name_is("jq"))
            .expect("expected jq command fact");
        assert_eq!(jq.static_utility_name(), Some("jq"));

        let tail = facts
            .pipelines()
            .first()
            .and_then(|pipeline| pipeline.last_segment())
            .expect("expected pipeline tail");
        assert_eq!(tail.static_utility_name(), Some("kill"));
        assert!(tail.static_utility_name_is("kill"));
    });
}

#[test]
fn surface_facts_ignore_single_quoted_payload_text_in_expanding_heredocs() {
    let source = "\
#!/bin/sh
cat <<EOF
'$HOME' and '$(pwd)'
EOF
cat <<-EOF
\t'${USER}'
EOF
";

    with_facts(source, None, |_, facts| {
        assert!(
            facts.single_quoted_fragments().is_empty(),
            "expected heredoc payload quotes to stay out of single-quoted shell fragments"
        );
    });
}

#[test]
fn surface_facts_keep_command_context_for_here_string_operands() {
    let source = "\
#!/bin/bash
bash --init-file \"${BASH_IT?}/bash_it.sh\" -i <<< '_bash-it-flash-term \"${#BASH_IT_THEME}\" \"${BASH_IT_THEME}\"'
";

    with_facts(source, None, |_, facts| {
        let fragment = facts
            .single_quoted_fragments()
            .iter()
            .find(|fragment| {
                fragment
                    .span()
                    .slice(source)
                    .contains("_bash-it-flash-term")
            })
            .expect("expected here-string single-quoted fragment");

        assert_eq!(fragment.command_name(), Some("bash"));
    });
}

#[test]
fn surface_facts_do_not_apply_command_context_to_plain_redirect_targets() {
    let source = "\
#!/bin/bash
bash > '$HOME'
";

    with_facts(source, None, |_, facts| {
        let fragment = facts
            .single_quoted_fragments()
            .iter()
            .find(|fragment| fragment.span().slice(source) == "'$HOME'")
            .expect("expected redirect-target single-quoted fragment");

        assert_eq!(fragment.command_name(), None);
    });
}

#[test]
fn surface_facts_mark_single_quoted_backslash_sequences_that_continue_into_literals() {
    let source = "\
#!/bin/sh
grep ^start'\\s'end file.txt
printf '%s\\n' '\\n'foo
printf '%s\\n' 'ab\\n'c
printf '%s\\n' '\\\\n'foo
printf '%s\\n' 'foo\\nbar'
printf '%s\\n' '\\x'41
printf '%s\\n' '\\0'foo
printf '%s\\n' '\\n'_
printf '%s\\n' $'\\n'foo
printf '%s\\n' a'\\'bc
";

    with_facts(source, None, |_, facts| {
        let flagged = facts
            .single_quoted_fragments()
            .iter()
            .filter_map(|fragment| {
                fragment
                    .literal_backslash_in_single_quotes_span()
                    .map(|span| {
                        (
                            fragment.span().slice(source),
                            span.start.line,
                            span.start.column,
                        )
                    })
            })
            .collect::<Vec<_>>();

        assert_eq!(
            flagged,
            vec![
                ("'\\s'", 2, 15),
                ("'\\n'", 3, 18),
                ("'ab\\n'", 4, 20),
                ("'\\\\n'", 5, 19),
            ]
        );
    });
}

#[test]
fn positional_parameter_surface_facts_ignore_single_digit_suffixes_in_nested_substitutions() {
    let source = r#"#!/bin/sh
eval "$(printf '%s\n' x | "$2_rework")"
eval "$(printf '%s\n' x | "$10_rework")"
"#;

    with_facts(source, None, |_, facts| {
        let above_nine = facts
            .positional_parameter_fragments()
            .iter()
            .filter(|fragment| fragment.is_above_nine())
            .map(|fragment| fragment.span().slice(source))
            .collect::<Vec<_>>();

        assert_eq!(above_nine.len(), 1, "fragments: {above_nine:?}");
        assert!(above_nine[0].contains("$10"), "fragments: {above_nine:?}");
        assert!(
            !above_nine
                .iter()
                .any(|fragment| fragment.contains("$2_rework")),
            "fragments: {above_nine:?}"
        );
    });
}

#[test]
fn builds_nested_legacy_arithmetic_fragments_from_surface_words() {
    let source = "\
#!/bin/bash
echo $[$[1 + 2] + 3]
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .legacy_arithmetic_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["$[$[1 + 2] + 3]", "$[1 + 2]"]
        );
    });
}

#[test]
fn open_double_quote_surface_facts_track_live_expansion_gaps() {
    let source = "\
#!/bin/bash
echo \"#!/bin/bash

# LLVMFuzzerTestOneInput for fuzzer detection.
this_dir=\\$(dirname \"\\$0\")
if [[ \"\\$@\" =~ (^| )-runs=[0-9]+($| ) ]]
then
  mem_settings='-Xmx1900m:-Xss900k'
else
  mem_settings='-Xmx2048m:-Xss1024k'
fi

LD_LIBRARY_PATH=\"$JVM_LD_LIBRARY_PATH\":\\$this_dir \\
  \\$this_dir/jazzer_driver                        \\
  --agent_path=\\$this_dir/jazzer_agent_deploy.jar \\
  --cp=$RUNTIME_CLASSPATH                         \\
  --target_class=$fuzzer_basename                 \\
  --jvm_args=\"\\$mem_settings\"                     \\
  \\$@\" > $OUT/$fuzzer_basename
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(6, 11)]);
        assert_eq!(close, vec![(13, 17)]);
    });
}

#[test]
fn open_double_quote_surface_facts_ignore_escaped_literal_gaps() {
    let source = "\
#!/bin/bash
echo \"#!/bin/bash
# LLVMFuzzerTestOneInput for fuzzer detection.
this_dir=\\$(dirname \"\\$0\")
mem_settings='-Xmx2048m:-Xss1024k'
if [[ \"\\$@\" =~ (^| )-runs=[0-9]+($| ) ]]; then
  mem_settings='-Xmx1900m:-Xss900k'
fi
LD_LIBRARY_PATH=\\\"\\$JVM_LD_LIBRARY_PATH\\\":\\$this_dir \\
\\$this_dir/jazzer_driver --agent_path=\\$this_dir/jazzer_agent_deploy.jar \\
--cp=$RUNTIME_CLASSPATH \\
--target_class=$fuzzer_basename \\
--jvm_args=\"\\$mem_settings\" \\
\"\\$@\"\" > $OUT/$fuzzer_basename
";

    with_facts(source, None, |_, facts| {
        assert!(facts.open_double_quote_fragments().is_empty());
        assert!(facts.suspect_closing_quote_fragments().is_empty());
    });
}

#[test]
fn open_double_quote_surface_facts_track_literal_gap_fragments() {
    let source = "\
#!/bin/sh
echo \"help text
say \"configure\" now
\"
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 6)]);
        assert_eq!(close, vec![(3, 5)]);
        assert_eq!(
            facts.open_double_quote_fragments()[0].replacement(),
            "\"help text\nsay configure now\n\""
        );
    });
}

#[test]
fn open_double_quote_surface_facts_keep_assignment_fixes_value_scoped() {
    let source = "\
#!/bin/bash
value='alpha
beta''tail'
";

    with_facts(source, None, |_, facts| {
        let fragment = &facts.open_double_quote_fragments()[0];

        assert_eq!(
            fragment.replacement_span().slice(source),
            "'alpha\nbeta''tail'"
        );
        assert_eq!(fragment.replacement(), "\"alpha\nbetatail\"");
    });
}

#[test]
fn open_double_quote_surface_facts_track_backslash_prefixed_literal_gap_fragments() {
    let source = "\
#!/bin/sh
echo \"line one
line two\"\\foo\"tail\"
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 6)]);
        assert_eq!(close, vec![(3, 9)]);
    });
}

#[test]
fn open_double_quote_surface_facts_track_multiline_quotes_with_literal_suffix() {
    let source = "\
#!/bin/sh
echo \"\"\"line one
line two\"suffix
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 8)]);
        assert_eq!(close, vec![(3, 9)]);
    });
}

#[test]
fn open_double_quote_surface_facts_track_multiline_quotes_with_suffix_expansion() {
    let source = "\
#!/bin/sh
echo \"line one
line two\"$suffix
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 6)]);
        assert_eq!(close, vec![(3, 9)]);
    });
}

#[test]
fn open_double_quote_surface_facts_track_empty_prefix_multiline_quotes_with_suffix_expansion() {
    let source = "\
#!/bin/sh
echo \"\"\"line one
line two\"$suffix
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 8)]);
        assert_eq!(close, vec![(3, 9)]);
    });
}

#[test]
fn open_double_quote_surface_facts_track_reopened_single_quote_fragments() {
    let source = "\
#!/bin/sh
echo 'line one
line two''tail'
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 6)]);
        assert_eq!(close, vec![(3, 9)]);
    });
}

#[test]
fn open_double_quote_surface_facts_track_recovered_single_quoted_assignment_shape() {
    let source = "\
#!/bin/sh
archive_cmds='$CC -o $output_objdir/$soname $libobjs $compiler_flags $deplibs -Wl,-dll~linknames='
archive_expsym_cmds='if test \"x`$SED 1q $export_symbols`\" = xEXPORTS; then
    sed -n -e 's/\\\\\\\\\\\\\\(.*\\\\\\\\\\\\\\)/-link\\\\\\ -EXPORT:\\\\\\\\\\\\\\\u{1}/' -e '1\\\\\\!p' < $export_symbols > $output_objdir/$soname.exp;
  else
    sed -e 's/\\\\\\\\\\\\\\(.*\\\\\\\\\\\\\\)/-link\\\\\\ -EXPORT:\\\\\\\\\\\\\\\u{1}/' < $export_symbols > $output_objdir/$soname.exp;
  fi~
  $CC -o $tool_output_objdir$soname $libobjs $compiler_flags $deplibs \"@$tool_output_objdir$soname.exp\" -Wl,-DLL,-IMPLIB:\"$tool_output_objdir$libname.dll.lib\"~
  linknames='
enable_shared_with_static_runtimes=yes
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(3, 21), (4, 75)]);
        assert_eq!(close, vec![(4, 15), (6, 12)]);
    });
}

#[test]
fn open_double_quote_surface_facts_track_each_reopened_fragment_per_word() {
    let source = "\
#!/bin/sh
echo \"help text
say \"configure\" now
then \"install\" later
\"\"\"
";

    with_facts(source, None, |_, facts| {
        let open = facts
            .open_double_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();
        let close = facts
            .suspect_closing_quote_fragments()
            .iter()
            .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
            .collect::<Vec<_>>();

        assert_eq!(open, vec![(2, 6), (3, 15), (4, 14)]);
        assert_eq!(close, vec![(3, 5), (4, 6), (5, 1)]);
    });
}

#[test]
fn builds_double_paren_grouping_spans() {
    let source = "\
#!/bin/sh
((ps aux | grep foo) || kill \"$pid\") 2>/dev/null
(( i += 1 ))
";

    with_facts(source, None, |_, facts| {
        let anchors = facts
            .double_paren_grouping_spans()
            .iter()
            .map(|span| {
                (
                    span.start.line,
                    span.start.column,
                    span.end.line,
                    span.end.column,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(anchors, vec![(2, 1, 2, 1)]);
    });
}

#[test]
fn builds_unicode_smart_quote_spans_for_unquoted_words() {
    let source = "\
#!/bin/sh
echo “hello”
echo \"hello “world”\"
echo 'hello ‘world’'
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .unicode_smart_quote_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["“", "”"]
        );
    });
}

#[test]
fn ignores_unicode_smart_quotes_in_heredoc_payloads() {
    let source = "\
#!/bin/sh
cat <<EOF
q { quotes: \"“\" \"”\" \"‘\" \"’\"; }
EOF
";

    with_facts(source, None, |_, facts| {
        assert!(facts.unicode_smart_quote_spans().is_empty());
    });
}

#[test]
fn traces_case_pattern_spans_for_escaped_char_classes() {
    let source = "\
#!/bin/sh
case x in *[!a-zA-Z0-9._/+\\-]*) continue ;; esac
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .pattern_literal_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*[!a-zA-Z0-9._/+\\-]*"]
        );
        assert!(facts.pattern_charclass_spans().is_empty());
    });
}

#[test]
fn marks_suppressed_subscript_references_without_span_scanning() {
    let source = "\
#!/bin/bash
write_target() { map[$assoc_target_id/assoc_target_bare]=value; }
declare -A map
printf '%s\\n' \"${arr[$read_idx]}\" \"${map[$assoc_read_idx]}\" \"$free\"
[[ -v arr[bare_check] ]]
[[ -v arr[$dynamic_check] ]]
arr[bare_target]=value
arr[$dynamic_target]=value
arr+=([bare_key]=value)
arr+=([$dynamic_key]=value)
map+=([assoc_bare_key]=value)
map+=([$assoc_dynamic_key]=value)
write_target
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);

    let reference_span = |name: &str| {
        semantic
            .references()
            .iter()
            .find(|reference| reference.name.as_str() == name)
            .unwrap_or_else(|| panic!("expected {name} reference"))
            .span
    };

    assert!(facts.is_suppressed_subscript_reference(reference_span("read_idx")));
    assert!(facts.is_suppressed_subscript_reference(reference_span("assoc_read_idx")));
    assert!(facts.is_suppressed_subscript_reference(reference_span("bare_check")));
    assert!(facts.is_suppressed_subscript_reference(reference_span("assoc_target_bare")));
    assert!(facts.is_suppressed_subscript_reference(reference_span("assoc_bare_key")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("dynamic_check")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("bare_target")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("dynamic_target")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("bare_key")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("dynamic_key")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("assoc_target_id")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("assoc_dynamic_key")));
    assert!(!facts.is_suppressed_subscript_reference(reference_span("free")));

    assert!(facts.is_subscript_later_suppression_reference(reference_span("read_idx")));
    assert!(facts.is_subscript_later_suppression_reference(reference_span("assoc_read_idx")));
    assert!(!facts.is_subscript_later_suppression_reference(reference_span("bare_check")));
    assert!(!facts.is_subscript_later_suppression_reference(reference_span("dynamic_check")));
}

#[test]
fn collects_array_index_arithmetic_spans_only_for_indexed_lvalues() {
    let source = "\
#!/bin/bash
arr[$((indexed+1))]=x
declare named[$((declared+1))]=y
declare -A map
map[$((assoc+1))]=z
map[temp_$((mixed+1))]=q
map=([$((compound+1))]=w)
printf '%s\\n' \"${arr[$((read+1))]}\"
[[ -v arr[$((check+1))] ]]
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .array_index_arithmetic_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$((indexed+1))", "$((declared+1))"]
        );
    });
}

#[test]
fn tracks_env_prefix_scope_spans_for_same_command_references() {
    let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --with-optmizer=${CFLAGS}
PATH=/tmp \"$PATH\"/bin/tool
A=1 B=\"$A\" C=\"$B\" cmd
foo=\"$foo\" bar=\"$foo\" cmd
foo=1 export \"$foo\"
foo=1 bar[$foo]=x cmd
FOO=tmp cmd >\"$FOO\"
foo=\"$foo\" cmd
foo=1 cmd \"$(printf %s \"$foo\")\"
foo=1 foo=2 cmd
foo=1 bar=\"$foo\"
COUNTDOWN=$[ $COUNTDOWN - 1 ]
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo \"$COUNTDOWN\"
X=1 A=$[ $X + 1 ] true
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .env_prefix_assignment_scope_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "CFLAGS",
                "PATH",
                "A",
                "B",
                "foo",
                "foo",
                "foo",
                "FOO",
                "COUNTDOWN",
                "X"
            ]
        );
        assert_eq!(
            facts
                .env_prefix_expansion_scope_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${CFLAGS}",
                "$PATH",
                "$A",
                "$B",
                "$foo",
                "$foo",
                "$foo",
                "$FOO",
                "$COUNTDOWN",
                "$X"
            ]
        );
    });
}

#[test]
fn tracks_env_prefix_expansion_spans_for_duplicate_name_self_reference() {
    let source = "\
#!/bin/bash
foo=\"$foo\" foo=2 cmd
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .env_prefix_expansion_scope_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    });
}

#[test]
fn prompt_expansion_refs_do_not_create_env_prefix_scope_spans() {
    let source = "\
#!/bin/bash
x=1 declare PS1='$x'
x=1 export PS4=\"+ \\${x} \"
";

    with_facts(source, None, |_, facts| {
        assert!(
            facts.env_prefix_assignment_scope_spans().is_empty(),
            "unexpected assignment scope spans: {:?}",
            facts
                .env_prefix_assignment_scope_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
        assert!(
            facts.env_prefix_expansion_scope_spans().is_empty(),
            "unexpected expansion scope spans: {:?}",
            facts
                .env_prefix_expansion_scope_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    });
}

#[test]
fn builds_word_facts_with_contexts_hosts_and_anchor_spans() {
    let source = "\
#!/bin/bash
case literal in
  @($pat|$(printf '%s' case))) : ;;
esac
trap \"echo $x $(date)\" EXIT
trap - ${signals[@]}
declare declared[$(printf decl-name-subscript)]
declare arr[$(printf decl-subscript)]=\"${name%$suffix}\"
target[$(printf assign-subscript)]=1
declare -A map=([$(printf key-subscript)]=1)
[[ -v assoc[\"$(printf cond-subscript)\"] ]]
printf '%s\\n' prefix${name}suffix ${items[@]}
";

    with_facts(source, None, |_, facts| {
        let case_subject = facts
            .case_subject_facts()
            .find(|fact| fact.span().slice(source) == "literal")
            .expect("expected case subject fact");
        assert!(case_subject.is_case_subject());
        assert!(case_subject.classification().is_fixed_literal());

        let trap_action = facts
            .expansion_word_facts(ExpansionContext::TrapAction)
            .next()
            .expect("expected trap action fact");
        assert_eq!(
            trap_action
                .double_quoted_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)"]
        );

        let trap_signal = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source) == "${signals[@]}")
            .expect("expected trap signal argument fact");
        assert_eq!(
            trap_signal
                .unquoted_all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${signals[@]}"]
        );

        let declaration_name_subscript = facts
            .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
            .find(|fact| fact.span().slice(source) == "$(printf decl-name-subscript)")
            .expect("expected declaration name subscript fact");
        assert_eq!(
            declaration_name_subscript.host_kind(),
            WordFactHostKind::DeclarationNameSubscript
        );
        assert_eq!(
            declaration_name_subscript
                .command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf decl-name-subscript)"]
        );

        let declaration_assignment_subscript = facts
            .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
            .find(|fact| fact.span().slice(source) == "$(printf decl-subscript)")
            .expect("expected declaration assignment subscript fact");
        assert_eq!(
            declaration_assignment_subscript.host_kind(),
            WordFactHostKind::AssignmentTargetSubscript
        );

        let assignment_subscript = facts
            .expansion_word_facts(ExpansionContext::AssignmentValue)
            .find(|fact| fact.span().slice(source) == "$(printf assign-subscript)")
            .expect("expected assignment subscript fact");
        assert_eq!(
            assignment_subscript.host_kind(),
            WordFactHostKind::AssignmentTargetSubscript
        );

        let array_key = facts
            .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
            .find(|fact| fact.span().slice(source) == "$(printf key-subscript)")
            .expect("expected array key fact");
        assert_eq!(array_key.host_kind(), WordFactHostKind::ArrayKeySubscript);

        let conditional_subscript = facts
            .expansion_word_facts(ExpansionContext::ConditionalVarRefSubscript)
            .find(|fact| fact.span().slice(source) == "\"$(printf cond-subscript)\"")
            .expect("expected conditional subscript fact");
        assert_eq!(
            conditional_subscript.host_kind(),
            WordFactHostKind::ConditionalVarRefSubscript
        );

        let parameter_pattern = facts
            .expansion_word_facts(ExpansionContext::ParameterPattern)
            .find(|fact| fact.span().slice(source) == "$suffix")
            .expect("expected parameter pattern fact");
        assert!(parameter_pattern.classification().is_expanded());
        assert_eq!(
            facts
                .expansion_word_facts(ExpansionContext::ParameterPattern)
                .filter(|fact| fact.span().slice(source) == "$suffix")
                .count(),
            1
        );

        let scalar = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source) == "prefix${name}suffix")
            .expect("expected mixed command argument fact");
        assert!(scalar.has_literal_affixes());
        assert_eq!(
            scalar
                .scalar_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}"]
        );

        let array = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source) == "${items[@]}")
            .expect("expected array argument fact");
        assert_eq!(
            array
                .unquoted_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${items[@]}"]
        );
    });
}

#[test]
fn builds_impossible_case_pattern_spans_from_subject_shape() {
    let source = "\
#!/bin/sh
case \"$words[1]\" in
  install) : ;;
  *\"[1]\") : ;;
esac
case \" $oldobjs \" in
  \" \") : ;;
  \"  \") : ;;
esac
case foo in
  bar) : ;;
esac
case \"x${val}z\" in
  y*) : ;;
  *z) : ;;
esac
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .case_pattern_impossible_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["install", "\" \"", "y*"]
        );
    });
}

#[test]
fn word_facts_track_only_literal_command_trailers() {
    let source = "\
#!/bin/bash
\"$PID exists.\"
\"$root/bin/{{\"
\"${loader:?}\"
\"/usr/bin/qemu-${machine}\"
\"$(printf cmd)\"
";

    with_facts(source, None, |_, facts| {
        let trailing_literal_char = |text: &str| {
            facts
                .word_facts()
                .find(|fact| fact.span().slice(source) == text)
                .and_then(|fact| fact.trailing_literal_char())
        };

        assert_eq!(trailing_literal_char("\"$PID exists.\""), Some('.'));
        assert_eq!(trailing_literal_char("\"$root/bin/{{\""), Some('{'));
        assert_eq!(trailing_literal_char("\"${loader:?}\""), None);
        assert_eq!(trailing_literal_char("\"/usr/bin/qemu-${machine}\""), None);
        assert_eq!(trailing_literal_char("\"$(printf cmd)\""), None);
    });
}

#[test]
fn builds_case_pattern_expansion_spans_for_simple_dynamic_patterns() {
    let source = "\
#!/bin/sh
case $value in
  $pat) : ;;
  x$pat) : ;;
  $(printf '%s' foo)) : ;;
  \"$left\"$right) : ;;
esac
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .case_pattern_expansions()
                .iter()
                .map(|fact| (fact.span().slice(source), fact.replacement().to_owned()))
                .collect::<Vec<_>>(),
            vec![
                ("$pat", "\"${pat}\"".to_owned()),
                ("x$pat", "\"x${pat}\"".to_owned()),
                ("$(printf '%s' foo)", "\"$(printf '%s' foo)\"".to_owned()),
                ("\"$left\"$right", "\"${left}${right}\"".to_owned()),
            ]
        );
    });
}

#[test]
fn ignores_case_pattern_expansions_when_real_globs_are_present() {
    let source = "\
#!/bin/bash
case $value in
  gm$MAMEVER*) : ;;
  *${IDN_ITEM}*) : ;;
  ${pat}*) : ;;
  *${pat}) : ;;
  x${pat}*) : ;;
  [$hex]) : ;;
  @($pat|bar)) : ;;
  x$left@(foo|bar)) : ;;
esac
";

    with_facts(source, None, |_, facts| {
        assert!(facts.case_pattern_expansions().is_empty());
    });
}

#[test]
fn collects_dollar_spans_for_wrapped_substring_offset_arithmetic() {
    let source = "#!/bin/bash\nrest=abcdef\nlen=2\nprintf '%s\\n' \"${rest:$((${#rest}-$len))}\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        let words = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .map(|fact| {
                format!(
                    "{} {:?} {:?}",
                    fact.span().slice(source),
                    fact.host_kind(),
                    fact.word().parts
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$len"], "command words: {words:?}");
    });
}

#[test]
fn collects_dollar_spans_for_wrapped_substring_length_arithmetic() {
    let source =
        "#!/bin/bash\nstring=abcdef\nwidth=10\nprintf '%s\\n' \"${string:0:$(( $width - 4 ))}\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        let words = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .map(|fact| {
                format!(
                    "{} {:?} {:?}",
                    fact.span().slice(source),
                    fact.host_kind(),
                    fact.word().parts
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$width"], "command words: {words:?}");
    });
}

#[test]
fn ignores_plain_substring_offset_parameter_expansions_for_dollar_in_arithmetic() {
    let source = "#!/bin/bash\nrest=abcdef\nlen=2\nprintf '%s\\n' \"${rest:${len}:1}\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_plain_positional_slice_parameter_expansions_for_dollar_in_arithmetic() {
    let source = "#!/bin/bash\nargs_offset=$#\nprintf '%s\\n' \"${@:1:${args_offset}}\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn collects_dollar_spans_for_parameter_replacement_arithmetic() {
    let source = "#!/bin/bash\noffset=1\nindex=2\nline=x\necho \"${line/ $index / $(($offset + $index)) }\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        let words = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .map(|fact| {
                format!(
                    "{} {:?} {:?}",
                    fact.span().slice(source),
                    fact.host_kind(),
                    fact.word().parts
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$offset", "$index"], "command words: {words:?}");
    });
}

#[test]
fn collects_dollar_spans_for_simple_subscripted_parameter_accesses_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -a arr
declare -A assoc
(( ${arr[0]} + ${arr[i]} + ${assoc[key]} ))
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["${arr[0]}", "${arr[i]}", "${assoc[key]}"]);
    });
}

#[test]
fn collects_dollar_spans_for_wrapped_substring_offset_with_simple_subscripted_access() {
    let source = "\
#!/bin/bash
arr=(0 1)
rest=abcdef
printf '%s\\n' \"${rest:$((${arr[0]}))}\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["${arr[0]}"]);
    });
}

#[test]
fn collects_dollar_spans_for_indexed_assignment_subscripts() {
    let source = "\
#!/bin/bash
declare -a arr
i=1
lang=en
arr[$i]=x
arr[$i+1]=y
arr[$i/repo_dir]=z
arr[${lang},27]=q
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$i", "$i", "$i", "${lang}"]);
    });
}

#[test]
fn ignores_associative_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -A assoc
key=name
lang=en
assoc[$key]=x
assoc[${lang},27]=y
assoc[$key/sfx]=z
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_quoted_indexed_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -a arr
wash_counter=1
arr[\"${wash_counter}\"]=x
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_command_substitutions_in_indexed_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -a arr
i=file
arr[$(printf '%s' \"$i\")]=x
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_multi_declared_associative_append_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -A one=() two=() seen=()
key=name
one[$key]+=x
two[$key]+=y
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_associative_appends_after_parameter_default_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -A map=() other=()
key=name
: \"${map[$key]:=}\"
map[$key]+=$'\\n'
map[$key]+=\"${values[*]}\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_globally_declared_associative_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
init() {
  declare -gA map
}
helper() {
  map[$key]=1
}
main() {
  key=name
  init
  helper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_globally_declared_associative_assignment_subscripts_with_combined_flags() {
    let source = "\
#!/bin/bash
init() {
  declare -Ag map=()
}
helper() {
  map[$key/field]=1
}
main() {
  key=name
  init
  helper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_dynamic_scope_associative_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
helper() {
  map[${key}]=1
}
wrapper() {
  helper
}
main() {
  local key=name
  declare -A map
  wrapper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn reports_shadowing_local_subscripts_even_when_callers_have_assoc_bindings() {
    let source = "\
#!/bin/bash
helper() {
  local map
  map[$key]=1
}
main() {
  local key=name
  declare -A map
  helper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$key"]);
    });
}

#[test]
fn reports_caller_local_shadowing_after_assoc_declaration_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
helper() {
  map[$key]=1
}
main() {
  local key=name
  declare -A map
  unset map
  local map
  helper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$key"]);
    });
}

#[test]
fn ignores_repeated_dynamic_scope_associative_assignment_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
helper() {
  map[${key}]=1
  map[${other}]=2
}
main() {
  local key=alpha
  local other=beta
  declare -A map
  helper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn reports_wrapper_shadowing_local_subscripts_even_when_outer_callers_have_assoc_bindings() {
    let source = "\
#!/bin/bash
helper() {
  map[$key]=1
}
wrapper() {
  local map
  helper
}
main() {
  local key=name
  declare -A map
  wrapper
}
main
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$key"]);
    });
}

#[test]
fn ignores_associative_declaration_initializer_subscripts_for_dollar_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -A map=([$key]=1)
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn collects_dollar_spans_for_nested_arithmetic_in_array_access_subscripts() {
    let source = "\
#!/bin/bash
declare -a tools
choice=2
printf '%s\\n' \"${tools[$(($choice-1))]}\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$choice"]);
    });
}

#[test]
fn collects_dollar_spans_for_nested_arithmetic_in_associative_assignment_subscripts() {
    let source = "\
#!/bin/bash
declare -A assoc
choice=2
assoc[$(($choice-1))]=x
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$choice"]);
    });
}

#[test]
fn collects_command_substitution_spans_for_wrapped_substring_offset_arithmetic() {
    let source = "#!/bin/bash\nrest=abcdef\nprintf '%s\\n' \"${rest:$((${#rest}-$(printf 1)))}\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .arithmetic_command_substitution_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$(printf 1)"]);
    });
}

#[test]
fn word_facts_descend_into_arithmetic_command_substitution_bodies() {
    let source = "\
#!/bin/bash
if (( $(du -c $mask | cut -f 1) == 0 )); then
  :
fi
for (( i=$(next $start); i < 3; i++ )); do
  :
done
";

    with_facts(source, None, |_, facts| {
        let nested_args = facts
            .word_facts()
            .filter(|fact| fact.is_nested_word_command())
            .filter(|fact| fact.host_expansion_context() == Some(ExpansionContext::CommandArgument))
            .map(|fact| fact.span().slice(source))
            .collect::<Vec<_>>();

        assert!(nested_args.contains(&"$mask"), "{nested_args:?}");
        assert!(nested_args.contains(&"$start"), "{nested_args:?}");
    });
}

#[test]
fn ignores_quoted_dollar_words_in_arithmetic_command_contexts() {
    let source = "#!/bin/bash\n(( \"$x\" + 1 ))\nfor (( i=\"$y\"; i < 3; i++ )); do :; done\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn indexes_pending_arithmetic_word_facts_by_span() {
    let source = "#!/bin/bash\nprintf '%s\\n' $(( $name + 1 ))\n";

    with_facts(source, None, |_, facts| {
        let arithmetic = facts
            .arithmetic_command_word_facts()
            .find(|fact| fact.span().slice(source) == "$name")
            .expect("expected arithmetic word fact");

        assert!(arithmetic.is_arithmetic_command());
        assert_eq!(
            facts
                .word_fact(arithmetic.span(), arithmetic.context())
                .map(|fact| fact.span().slice(source)),
            Some("$name")
        );
        assert_eq!(
            facts
                .any_word_fact(arithmetic.span())
                .map(|fact| fact.span().slice(source)),
            Some("$name")
        );
    });
}

#[test]
fn indexes_arithmetic_word_facts_inside_parameter_replacement_operands() {
    let source = "#!/bin/bash\nprintf '%s\\n' \"${value/foo/$(( $name + 1 ))}\"\n";

    with_facts(source, None, |_, facts| {
        let arithmetic = facts
            .arithmetic_command_word_facts()
            .find(|fact| fact.span().slice(source) == "$name")
            .expect("expected arithmetic word fact");

        assert!(arithmetic.is_arithmetic_command());
        assert_eq!(
            arithmetic.host_expansion_context(),
            Some(ExpansionContext::CommandArgument)
        );
        assert_eq!(
            facts
                .word_fact(arithmetic.span(), arithmetic.context())
                .map(|fact| fact.span().slice(source)),
            Some("$name")
        );
    });
}

#[test]
fn indexes_arithmetic_word_facts_inside_parameter_default_operands() {
    let source = "\
#!/bin/bash
printf '%s\\n' \"${value:-$(( $default + 1 ))}\" \"${value:=$(( $assign + 1 ))}\" \"${value:+$(( $replace + 1 ))}\" \"${value:?$(( $error + 1 ))}\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .arithmetic_command_word_facts()
            .map(|fact| fact.span().slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$default", "$assign", "$replace", "$error"]);
        assert!(facts.arithmetic_command_word_facts().all(|fact| {
            fact.host_expansion_context() == Some(ExpansionContext::CommandArgument)
                && facts.word_fact(fact.span(), fact.context()).is_some()
        }));
    });
}

#[test]
fn ignores_dynamic_and_compound_subscript_parameter_accesses_in_arithmetic() {
    let source = "\
#!/bin/bash
declare -a arr
declare -A assoc
i=0
key=name
(( ${arr[$i]} + ${arr[i+1]} + ${arr[-1]} + ${assoc[$key]} ))
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .dollar_in_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn ignores_escaped_command_substitution_tokens_in_wrapped_substring_offset_arithmetic() {
    let source = "#!/bin/bash\ns=abcdef\ni=1\nprintf '%s\\n' \"${s:$(($i+\\$(printf 1)))}\"\n";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .arithmetic_command_substitution_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    });
}

#[test]
fn builds_word_facts_for_zsh_qualified_globs() {
    let source = "#!/usr/bin/env zsh\nprint -- prefix*(.N)\n";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let glob = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "prefix*(.N)")
                .expect("expected zsh glob fact");

            assert!(glob.classification().is_expanded());
            assert!(glob.analysis().hazards.pathname_matching);
        },
    );
}

#[test]
fn builds_option_sensitive_word_behaviors_for_zsh_words() {
    let source = "\
#!/usr/bin/env zsh
setopt sh_word_split
print $name
noglob rm *
print ${~~name}
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let split = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "$name")
                .expect("expected split-sensitive fact");
            let wrapped_glob = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "*")
                .expect("expected wrapped glob fact");
            let double_tilde = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "${~~name}")
                .expect("expected double-tilde fact");

            assert_eq!(
                split.analysis().field_splitting_behavior,
                FieldSplittingBehavior::UnquotedOnly
            );
            assert_eq!(
                wrapped_glob.runtime_literal().pathname_expansion_behavior,
                PathnameExpansionBehavior::Disabled
            );
            assert_eq!(
                double_tilde.analysis().pathname_expansion_behavior,
                PathnameExpansionBehavior::LiteralGlobsOnly
            );
        },
    );
}

#[test]
fn builds_ambiguous_pathname_behaviors_for_dynamic_zsh_words() {
    let source = "\
#!/usr/bin/env zsh
opt=glob_subst
setopt \"$opt\"
print $name
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let ambiguous = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "$name")
                .expect("expected ambiguous scalar fact");

            assert_eq!(
                ambiguous.analysis().pathname_expansion_behavior,
                PathnameExpansionBehavior::Ambiguous
            );
            assert!(ambiguous.analysis().hazards.pathname_matching);
        },
    );
}

#[test]
fn builds_glob_subst_for_list_fanout_facts_for_zsh_words() {
    let source = "\
#!/usr/bin/env zsh
setopt glob_subst
for item in $name; do :; done
setopt no_glob
for item in $literal; do :; done
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let glob_subst = facts
                .expansion_word_facts(ExpansionContext::ForList)
                .find(|fact| fact.span().slice(source) == "$name")
                .expect("expected glob_subst for-list fact");
            let no_glob = facts
                .expansion_word_facts(ExpansionContext::ForList)
                .find(|fact| fact.span().slice(source) == "$literal")
                .expect("expected no_glob for-list fact");

            assert_eq!(
                glob_subst.analysis().pathname_expansion_behavior,
                PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted
            );
            assert!(glob_subst.analysis().hazards.pathname_matching);
            assert!(glob_subst.analysis().can_expand_to_multiple_fields);
            assert_eq!(
                no_glob.analysis().pathname_expansion_behavior,
                PathnameExpansionBehavior::Disabled
            );
            assert!(!no_glob.analysis().hazards.pathname_matching);
            assert!(!no_glob.analysis().can_expand_to_multiple_fields);
        },
    );
}

#[test]
fn builds_flow_merged_literal_only_pathname_behaviors_for_zsh_words() {
    let source = "\
#!/usr/bin/env zsh
if cond; then
  setopt no_glob
fi
print $name
rm *
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let scalar = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "$name")
                .expect("expected scalar fact");
            let glob = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "*")
                .expect("expected glob fact");

            assert_eq!(
                scalar.analysis().pathname_expansion_behavior,
                PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled
            );
            assert!(!scalar.analysis().hazards.pathname_matching);
            assert_eq!(
                glob.runtime_literal().pathname_expansion_behavior,
                PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled
            );
            assert!(
                glob.runtime_literal()
                    .pathname_expansion_behavior
                    .literal_globs_can_expand()
            );
        },
    );
}

#[test]
fn builds_glob_failure_behaviors_for_zsh_globs() {
    let source = "\
#!/usr/bin/env zsh
setopt null_glob csh_null_glob
rm *
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let glob = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "*")
                .expect("expected glob fact");

            assert!(
                glob.runtime_literal()
                    .pathname_expansion_behavior
                    .literal_globs_can_expand()
            );
            assert_eq!(
                glob.runtime_literal().glob_failure_behavior,
                GlobFailureBehavior::CshNullGlob
            );
        },
    );
}

#[test]
fn builds_glob_dot_and_pattern_behaviors_for_zsh_globs() {
    let source = "\
#!/usr/bin/env zsh
rm *
setopt glob_dots extended_glob
rm *
setopt ksh_glob sh_glob
rm *
unsetopt extended_glob ksh_glob sh_glob
rm *
opt=glob_dots
unsetopt \"$opt\"
rm *
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let glob_behaviors = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .filter(|fact| fact.span().slice(source) == "*")
                .map(|fact| {
                    let literal = fact.runtime_literal();
                    let pattern = literal.glob_pattern_behavior;
                    (
                        literal.glob_dot_behavior,
                        pattern.extended_glob(),
                        pattern.ksh_glob(),
                        pattern.sh_glob(),
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                glob_behaviors,
                vec![
                    (
                        GlobDotBehavior::ExplicitDotRequired,
                        PatternOperatorBehavior::Disabled,
                        PatternOperatorBehavior::Disabled,
                        PatternOperatorBehavior::Disabled,
                    ),
                    (
                        GlobDotBehavior::DotfilesIncluded,
                        PatternOperatorBehavior::Enabled,
                        PatternOperatorBehavior::Disabled,
                        PatternOperatorBehavior::Disabled,
                    ),
                    (
                        GlobDotBehavior::DotfilesIncluded,
                        PatternOperatorBehavior::Enabled,
                        PatternOperatorBehavior::Enabled,
                        PatternOperatorBehavior::Enabled,
                    ),
                    (
                        GlobDotBehavior::DotfilesIncluded,
                        PatternOperatorBehavior::Disabled,
                        PatternOperatorBehavior::Disabled,
                        PatternOperatorBehavior::Disabled,
                    ),
                    (
                        GlobDotBehavior::Ambiguous,
                        PatternOperatorBehavior::Ambiguous,
                        PatternOperatorBehavior::Ambiguous,
                        PatternOperatorBehavior::Ambiguous,
                    ),
                ]
            );
        },
    );
}

#[test]
fn builds_word_facts_for_special_parameter_arguments() {
    let source = "\
#!/bin/bash
printf '%s\\n' $0 $1 $* $@
";

    with_facts(source, None, |_, facts| {
        let argument_words = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .map(|fact| fact.span().slice(source).to_owned())
            .collect::<Vec<_>>();

        assert!(argument_words.contains(&"$0".to_owned()));
        assert!(argument_words.contains(&"$1".to_owned()));
        assert!(argument_words.contains(&"$*".to_owned()));
        assert!(argument_words.contains(&"$@".to_owned()));
    });
}

#[test]
fn builds_word_facts_for_filename_builder_command_substitutions() {
    let source = "\
#!/bin/bash
/sbin/makepkg -l y -c n $OUTPUT/$PRGNAM-$VERSION\\_$(echo ${KERNEL} | tr '-' '_')-$ARCH-$BUILD$TAG.$PKGTYPE
";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| {
                fact.span()
                    .slice(source)
                    .contains("$(echo ${KERNEL} | tr '-' '_')")
            })
            .expect("expected makepkg output argument fact");

        assert_eq!(
            fact.unquoted_command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(echo ${KERNEL} | tr '-' '_')"],
            "parts: {:?}",
            fact.word().parts
        );
        assert_eq!(
            fact.split_sensitive_unquoted_command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(echo ${KERNEL} | tr '-' '_')"],
            "parts: {:?}",
            fact.word().parts
        );
    });
}

#[test]
fn builds_word_facts_for_docker_inspect_command_substitutions() {
    let source = "\
#!/bin/bash
docker inspect -f '{{ if ne \"true\" (index .Config.Labels \"com.dokku.devcontainer\") }}{{.ID}} {{ end }}' $(docker ps -q)
";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source) == "$(docker ps -q)")
            .expect("expected docker inspect argument fact");

        assert_eq!(
            fact.unquoted_command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(docker ps -q)"],
            "parts: {:?}",
            fact.word().parts
        );
    });
}

#[test]
fn builds_plain_unindexed_array_reference_facts_in_bash() {
    let source = "\
#!/bin/bash
arr=(one two)
printf '%s\\n' $arr \"${arr}\" pre${arr}post \"${arr[0]}\" \"${arr[@]}\" \"${arr%one}\"
cat <<EOF
$arr
${arr}
${arr[0]}
EOF
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .plain_unindexed_array_references()
                .map(|fact| match fact {
                    PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                        ("selector", reference.diagnostic_span().slice(source))
                    }
                    PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                        (
                            "native-zsh-scalar",
                            reference.expansion_span().slice(source),
                        )
                    }
                    PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                        ("ambiguous", reference.diagnostic_span().slice(source))
                    }
                })
                .collect::<Vec<_>>(),
            vec![
                ("selector", "$arr"),
                ("selector", "${arr}"),
                ("selector", "${arr}"),
                ("selector", "$arr"),
                ("selector", "${arr}"),
            ]
        );
    });
}

#[test]
fn plain_unindexed_array_references_classify_native_zsh_scalar_expansions() {
    let source = "\
#!/bin/zsh
arr=(one two)
print -r -- $arr
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(
                facts
                    .plain_unindexed_array_references()
                    .map(|fact| match fact {
                        PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                            ("selector", reference.diagnostic_span().slice(source))
                        }
                        PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                            (
                                "native-zsh-scalar",
                                reference.expansion_span().slice(source),
                            )
                        }
                        PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                            ("ambiguous", reference.diagnostic_span().slice(source))
                        }
                    })
                    .collect::<Vec<_>>(),
                vec![("native-zsh-scalar", "$arr")]
            );
        },
    );
}

#[test]
fn plain_unindexed_array_references_classify_setopt_ksh_arrays_expansions() {
    let source = "\
#!/bin/zsh
setopt ksh_arrays
arr=(one two)
print -r -- $arr
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(
                facts
                    .plain_unindexed_array_references()
                    .map(|fact| match fact {
                        PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                            ("selector", reference.diagnostic_span().slice(source))
                        }
                        PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                            (
                                "native-zsh-scalar",
                                reference.expansion_span().slice(source),
                            )
                        }
                        PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                            ("ambiguous", reference.diagnostic_span().slice(source))
                        }
                    })
                    .collect::<Vec<_>>(),
                vec![("selector", "$arr")]
            );
        },
    );
}

#[test]
fn plain_unindexed_array_references_classify_emulate_ksh_expansions() {
    let source = "\
#!/bin/zsh
emulate ksh
arr=(one two)
print -r -- $arr
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(
                facts
                    .plain_unindexed_array_references()
                    .map(|fact| match fact {
                        PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                            ("selector", reference.diagnostic_span().slice(source))
                        }
                        PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                            (
                                "native-zsh-scalar",
                                reference.expansion_span().slice(source),
                            )
                        }
                        PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                            ("ambiguous", reference.diagnostic_span().slice(source))
                        }
                    })
                    .collect::<Vec<_>>(),
                vec![("selector", "$arr")]
            );
        },
    );
}

#[test]
fn plain_unindexed_array_references_classify_dynamic_option_names_as_ambiguous() {
    let source = "\
#!/bin/zsh
opt=ksh_arrays
setopt \"$opt\"
arr=(one two)
print -r -- $arr
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(
                facts
                    .plain_unindexed_array_references()
                    .map(|fact| match fact {
                        PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                            ("selector", reference.diagnostic_span().slice(source))
                        }
                        PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                            (
                                "native-zsh-scalar",
                                reference.expansion_span().slice(source),
                            )
                        }
                        PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                            ("ambiguous", reference.diagnostic_span().slice(source))
                        }
                    })
                    .collect::<Vec<_>>(),
                vec![("ambiguous", "$arr")]
            );
        },
    );
}

#[test]
fn plain_unindexed_array_references_classify_dynamic_function_calls() {
    let source = "\
#!/bin/zsh
enable_ksh() {
  emulate ksh
}
dispatcher=enable_ksh
$dispatcher
arr=(one two)
print -r -- $arr
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(
                facts
                    .plain_unindexed_array_references()
                    .map(|fact| match fact {
                        PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                            ("selector", reference.diagnostic_span().slice(source))
                        }
                        PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                            (
                                "native-zsh-scalar",
                                reference.expansion_span().slice(source),
                            )
                        }
                        PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                            ("ambiguous", reference.diagnostic_span().slice(source))
                        }
                    })
                    .collect::<Vec<_>>(),
                vec![("selector", "$arr")]
            );
        },
    );
}

#[test]
fn plain_unindexed_array_references_classify_ambiguous_function_local_state() {
    let source = "\
#!/bin/zsh
f() {
  if [[ -n $flag ]]; then
    emulate -LR zsh
  fi
  local -a arr=(one two)
  print -r -- $arr
}
fn=f
setopt ksh_arrays
$fn
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(
                facts
                    .plain_unindexed_array_references()
                    .map(|fact| match fact {
                        PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                            ("selector", reference.diagnostic_span().slice(source))
                        }
                        PlainUnindexedArrayReferenceFact::NativeZshScalar(reference) => {
                            (
                                "native-zsh-scalar",
                                reference.expansion_span().slice(source),
                            )
                        }
                        PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                            ("ambiguous", reference.diagnostic_span().slice(source))
                        }
                    })
                    .collect::<Vec<_>>(),
                vec![("ambiguous", "$arr")]
            );
        },
    );
}

#[test]
fn builds_word_facts_for_quoted_all_elements_array_expansions() {
    let source = "\
#!/bin/bash
eval \"${shims[@]}\"
";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source) == "\"${shims[@]}\"")
            .expect("expected eval argument fact");

        assert_eq!(
            fact.all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${shims[@]}"],
            "parts: {:?}",
            fact.word().parts
        );
    });
}

#[test]
fn builds_word_facts_for_conditional_patterns() {
    let source = "\
#!/bin/bash
if [[ x == *${shims[@]}* ]]; then :; fi
";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::ConditionalPattern)
            .find(|fact| fact.span().slice(source) == "${shims[@]}")
            .expect("expected conditional pattern word fact");

        assert_eq!(
            fact.all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${shims[@]}"]
        );
        assert_eq!(
            fact.direct_all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${shims[@]}"],
            "parts: {:?}",
            fact.word().parts
        );
    });
}

#[test]
fn builds_word_facts_for_mixed_quoted_all_elements_array_expansions() {
    let source = "\
#!/bin/bash
shims=(a)
eval \"conda_shim() { case \\\"\\${1##*/}\\\" in ${shims[@]} *) return 1;; esac }\"
";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source).contains("${shims[@]}"))
            .expect("expected eval argument fact");

        assert_eq!(
            fact.all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${shims[@]}"]
        );
        assert_eq!(
            fact.direct_all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${shims[@]}"],
            "parts: {:?}",
            fact.word().parts
        );
    });
}

#[test]
fn direct_all_elements_word_facts_ignore_nested_positional_forwarding_idioms() {
    let source = "\
#!/bin/sh
eval shellspec_join SHELLSPEC_EXPECTATION '\" \"' The ${1+'\"$@\"'}
";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .find(|fact| fact.span().slice(source).contains("${1+'\"$@\"'}"))
            .expect("expected eval argument fact");

        assert_eq!(
            fact.all_elements_array_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(
            fact.direct_all_elements_array_expansion_spans().is_empty(),
            "parts: {:?}",
            fact.word().parts
        );
    });
}

#[test]
fn builds_word_facts_for_unquoted_all_elements_array_expansions() {
    let source = "\
#!/bin/bash
printf '%s\\n' $@ ${@:2} ${items[@]} ${items[@]:1} ${!items[@]} ${items[@]/#/#} ${items[@]@Q} ${items[@]:-fallback} ${items[@]:+fallback} \"$@\" \"${items[@]}\" $* ${items[*]} ${1+\"$@\"}
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .flat_map(|fact| {
                fact.unquoted_all_elements_array_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "$@",
                "${@:2}",
                "${items[@]}",
                "${items[@]:1}",
                "${!items[@]}",
                "${items[@]/#/#}",
                "${items[@]@Q}",
                "${items[@]:-fallback}"
            ]
        );
    });
}

#[test]
fn builds_word_facts_for_unquoted_literals_between_reopened_double_quotes() {
    let source = "\
#!/bin/bash
printf '%s\\n' \"foo\"bar\"baz\" \"foo\"-\"bar\" \"foo\"$(printf '%s' x)\"bar\" \"$left\"-\"$right\" x=\"$(cmd \"a\".\"b\")\" '$('\"foo\"parenmid\"baz\" '${'\"foo\"bracemid\"baz\" $(printf \"(\")\"foo\"quotedparen\"baz\" $(printf \"%s\" \"${x}\")\"foo\"quotedparam\"baz\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "bar",
                "-",
                "parenmid",
                "bracemid",
                "quotedparen",
                "quotedparam",
                "."
            ]
        );
    });
}

#[test]
fn builds_word_facts_skip_shellcheck_skipped_mixed_quote_literals() {
    let source = "\
#!/bin/bash
printf '%s\\n' \"foo\"*bar\"baz\" \"foo\"?bar\"baz\" \"foo\"a[b]\"baz\" \"foo\"a{b}\"baz\" \"foo\"a+b\"baz\" \"foo\"a@b\"baz\" \"foo\"user@host\"bar\"
export CARGO_TARGET_\"${env_host}\"_RUSTFLAGS+=\" -C\"
print \"\\
export EASYRSA_REQ_SERIAL=\\\"$EASYRSA_REQ_SERIAL\\\"\\
\" | sed -e s/a/b/
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "{spans:?}");
    });
}

#[test]
fn builds_word_facts_for_each_reopened_quote_line_join() {
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
param_hash=\"$AWK '\"\\
\"     {value=${value:- # fallback};\"\\
\"     /^COFF SYMBOL TABLE/{next};\"\\
\"     ' prfx=^$ac_symprfx\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::AssignmentValue)
            .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n", "\\\n",
                "\\\n", "\\\n", "\\\n", "\\\n", "\\\n"
            ]
        );
    });
}

#[test]
fn builds_word_facts_for_reopened_quote_line_join_after_word_span() {
    let source = "\
#!/bin/bash
value=\"foo\"\\
\"bar\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::AssignmentValue)
            .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["\\\n"]);
    });
}

#[test]
fn builds_word_facts_ignore_comment_text_in_nested_fragment_scan() {
    let source = "\
#!/bin/bash
echo $(echo x # $(
 )\"foo\"bar\"baz\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["bar"]);
    });
}

#[test]
fn builds_word_facts_ignore_comment_text_in_backtick_fragment_scan() {
    let source = "\
#!/bin/bash
echo `echo x # $(
`\"foo\"bar\"baz\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["bar"]);
    });
}

#[test]
fn builds_word_facts_ignore_hashes_inside_nested_double_quotes() {
    let source = "\
#!/bin/bash
echo $(printf \"%s\" \"x # $(printf y)\")\"foo\"bar\"baz\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["bar"]);
    });
}

#[test]
fn builds_word_facts_ignore_comment_text_in_process_substitution_scan() {
    let source = "\
#!/bin/bash
echo <(echo x # ${
 )\"foo\"bar\"baz\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .expansion_word_facts(ExpansionContext::CommandArgument)
            .flat_map(|fact| {
                fact.unquoted_literal_between_double_quoted_segments_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["bar"]);
    });
}

#[test]
fn builds_brace_variable_before_bracket_spans_from_direct_words() {
    let source = "\
#!/bin/bash
echo \"$foo[0]\"
echo \"${foo}[0]\"
echo \"$foo\"\"[0]\"
echo \"$foo\\[0]\"
$cmd[0] arg
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .brace_variable_before_bracket_spans()
            .iter()
            .map(|span| (span.start.line, span.start.column))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec![(2, 7), (6, 1)]);
    });
}

#[test]
fn builds_function_in_alias_spans_from_static_alias_definitions() {
    let source = "\
#!/bin/sh
alias first='echo $1'
alias rest='printf \"%s\\n\" \"$@\"'
alias conditional='${1+\"$@\"}'
alias escaped_then_pos='echo \\$$1'
alias func='helper() { echo hi; }'
alias literal='echo \\$1'
alias literal_braced='echo \\${1}'
alias quoted='echo '\"'\"'$1'\"'\"''
alias pid='echo $$1'
alias runtime=$BAR
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .function_in_alias_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "first='echo $1'",
                "rest='printf \"%s\\n\" \"$@\"'",
                "conditional='${1+\"$@\"}'",
                "escaped_then_pos='echo \\$$1'",
            ]
        );
    });
}

#[test]
fn builds_alias_definition_expansion_spans_without_matching_alias_lookups() {
    let source = "\
#!/bin/bash
alias \"${cur%=}\" 2>/dev/null
alias home=$HOME
alias \"$a=$b\"
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .alias_definition_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOME", "$a"]
        );
    });
}

#[test]
fn builds_array_assignment_split_word_facts() {
    let source = "\
#!/bin/bash
scalar=$x
arr=($x \"$y\" prefix$z $(cmd) \"${items[@]}\" ${items[@]})
declare declared=($alpha \"$(cmd)\" ${beta})
declare -A map=([k]=$v)
arr+=($tail)
";

    with_facts(source, None, |_, facts| {
        let split_words = facts
            .array_assignment_split_word_facts()
            .map(|fact| fact.span().slice(source).to_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            split_words,
            vec![
                "$x",
                "\"$y\"",
                "prefix$z",
                "$(cmd)",
                "\"${items[@]}\"",
                "${items[@]}",
                "$alpha",
                "\"$(cmd)\"",
                "${beta}",
                "$tail",
            ]
        );

        let unquoted_scalar = facts
            .array_assignment_split_word_facts()
            .flat_map(|fact| {
                fact.array_assignment_split_scalar_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            unquoted_scalar,
            vec!["$x", "$z", "$alpha", "${beta}", "$tail"]
        );

        let unquoted_array = facts
            .array_assignment_split_word_facts()
            .flat_map(|fact| {
                fact.unquoted_array_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(unquoted_array, vec!["${items[@]}"]);
    });
}

#[test]
fn array_assignment_split_facts_track_command_substitution_boundaries() {
    let source = "\
#!/bin/bash
arr=(\"$(printf '%s\\n' \"$x\")\")
";

    with_facts(source, None, |_, facts| {
        let split_facts = facts
            .array_assignment_split_word_facts()
            .collect::<Vec<_>>();
        assert_eq!(split_facts.len(), 1);
        let fact = split_facts[0];

        assert_eq!(
            fact.command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf '%s\\n' \"$x\")"]
        );
        assert_eq!(
            fact.array_assignment_split_scalar_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    });
}

#[test]
fn array_assignment_split_facts_reuse_inner_command_word_facts() {
    let source = "\
#!/bin/bash
arr=($(printf '%s\\n' \"$x\" ${y} prefix$z))
";

    with_facts(source, None, |_, facts| {
        let split_facts = facts
            .array_assignment_split_word_facts()
            .collect::<Vec<_>>();
        assert_eq!(split_facts.len(), 1);
        let fact = split_facts[0];

        assert_eq!(
            fact.span().slice(source),
            "$(printf '%s\\n' \"$x\" ${y} prefix$z)"
        );
        assert_eq!(
            fact.command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf '%s\\n' \"$x\" ${y} prefix$z)"]
        );
        assert_eq!(
            fact.array_assignment_split_scalar_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "${y}", "$z"]
        );
    });
}

#[test]
fn array_assignment_split_facts_keep_heredoc_substitutions_as_single_words() {
    let source = "\
#!/bin/bash
arr=(\"$(
  cat <<-EOF
    repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\")
EOF
)\")
";

    with_facts(source, None, |_, facts| {
        let split_words = facts
            .array_assignment_split_word_facts()
            .map(|fact| fact.span().slice(source).to_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            split_words,
            vec![
                "\"$(\n  cat <<-EOF\n    repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\")\nEOF\n)\""
            ]
        );
    });
}

#[test]
fn array_assignment_split_facts_keep_pipelined_heredoc_substitutions_as_single_words() {
    let source = "\
#!/bin/bash
arr=(\"$(
  cat <<-EOF | tr '\\n' ' '
    {
      \\\"query\\\": \\\"query {
        repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {
          refs(refPrefix: \\\"refs/tags/\\\")
        }
      }\\\"
    }
EOF
)\")
";

    with_facts(source, None, |_, facts| {
        let split_words = facts
            .array_assignment_split_word_facts()
            .map(|fact| fact.span().slice(source).to_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            split_words,
            vec![
                "\"$(\n  cat <<-EOF | tr '\\n' ' '\n    {\n      \\\"query\\\": \\\"query {\n        repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {\n          refs(refPrefix: \\\"refs/tags/\\\")\n        }\n      }\\\"\n    }\nEOF\n)\""
            ]
        );
    });
}

#[test]
fn array_assignment_split_facts_track_realistic_pipelined_heredoc_substitutions() {
    let source = r#"# shellcheck shell=bash
project=owner/repo
graphql_request=(
  -X POST
  -d "$(
    cat <<-EOF | tr '\n' ' '
      {
        "query": "query {
          repository(owner: \"${project%/*}\", name: \"${project##*/}\") {
            refs(refPrefix: \"refs/tags/\")
          }
        }"
      }
EOF
  )"
)
"#;

    with_facts(source, None, |_, facts| {
        let split_facts = facts
            .array_assignment_split_word_facts()
            .collect::<Vec<_>>();

        assert_eq!(
            split_facts
                .iter()
                .map(|fact| fact.span().slice(source))
                .collect::<Vec<_>>(),
            vec![
                "-X",
                "POST",
                "-d",
                "\"$(\n    cat <<-EOF | tr '\\n' ' '\n      {\n        \"query\": \"query {\n          repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {\n            refs(refPrefix: \\\"refs/tags/\\\")\n          }\n        }\"\n      }\nEOF\n  )\"",
            ]
        );
    });
}

#[test]
fn array_assignment_split_facts_ignore_use_replacement_expansions() {
    let source = "\
#!/bin/bash
arr=(${flag:+-f} ${flag:+$fallback} ${name:+\"$name\" \"$regex\"} ${items[@]+\"${items[@]}\"} ${x:-\"$fallback\"})
";

    with_facts(source, None, |_, facts| {
        let split_sensitive = facts
            .array_assignment_split_word_facts()
            .flat_map(|fact| {
                fact.array_assignment_split_scalar_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(split_sensitive, vec!["${x:-\"$fallback\"}"]);
    });
}

#[test]
fn array_assignment_split_facts_ignore_expansions_inside_brace_fanout() {
    let source = "\
#!/bin/bash
arr=({$XDG_CONFIG_HOME,$HOME}/{alacritty,}/{.,}alacritty.ym?)
arr=($prefix{a,b} {a,b}$suffix {pre$inside,other})
";

    with_facts(source, None, |_, facts| {
        let split_sensitive = facts
            .array_assignment_split_word_facts()
            .flat_map(|fact| {
                fact.array_assignment_split_scalar_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(split_sensitive, vec!["$prefix", "$suffix"]);
    });
}

#[test]
fn array_assignment_split_facts_keep_brace_literal_expansions_for_sh() {
    let source = "\
# shellcheck shell=sh
arr=({pre$inside,other})
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Bash,
        ShellDialect::Sh,
        |_, facts| {
            let split_sensitive = facts
                .array_assignment_split_word_facts()
                .flat_map(|fact| {
                    fact.array_assignment_split_scalar_expansion_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(split_sensitive, vec!["$inside"]);
        },
    );
}

#[test]
fn surface_facts_track_parameter_operations_in_expanding_heredocs() {
    let source = "\
cat <<EOF
${name:2}
${arr[0]//x/y}
${name^^pattern}
EOF
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .substring_expansion_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${name:2}"]
        );
        assert_eq!(
            facts
                .replacement_expansion_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]//x/y}"]
        );
        assert_eq!(
            facts
                .case_modification_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${name^^pattern}"]
        );
    });
}

#[test]
fn surface_facts_cover_replacement_expansions_with_escaped_backslashes() {
    let source = "\
#!/bin/sh
local crypt=$(grep \"^root:\" /etc/shadow | cut -f 2 -d :)
crypt=${crypt//\\\\/\\\\\\\\}
crypt=${crypt//\\//\\\\\\/}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .replacement_expansion_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${crypt//\\\\/\\\\\\\\}", "${crypt//\\//\\\\\\/}"]
        );
    });
}

#[test]
fn surface_facts_track_zsh_parameter_index_flags_only_for_word_targets() {
    let source = "\
#!/bin/sh
printf '%s\\n' ${\"$foo\"[1]}
printf '%s\\n' ${\"$(printf '%s\\n' \"$PWD\")\"[(w)1]}
printf '%s\\n' ${\"$(printf \"%s\" \")\")\"[(w)1]}
printf '%s\\n' ${map[(I)needle]}
printf '%s\\n' \"${precmd_functions[(r)_z_precmd]}\"
printf '%s\\n' '${\"$bar\"[1]}'
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .zsh_parameter_index_flag_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${\"$foo\"",
                "${\"$(printf '%s\\n' \"$PWD\")\"",
                "${\"$(printf \"%s\" \")\")\"",
            ]
        );
    });
}

#[test]
fn shared_command_traversal_collects_word_facts_and_surface_fragments() {
    let source = "\
#!/bin/bash
printf '%s\\n' ${name%$suffix} `printf backtick`
";

    with_facts(source, None, |_, facts| {
        let parameter_pattern = facts
            .expansion_word_facts(ExpansionContext::ParameterPattern)
            .find(|fact| fact.span().slice(source) == "$suffix")
            .expect("expected parameter pattern fact");
        assert_eq!(parameter_pattern.host_kind(), WordFactHostKind::Direct);

        assert_eq!(
            facts
                .backtick_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["`printf backtick`"]
        );
    });
}

#[test]
fn indexed_array_reference_fragments_include_operator_expansions() {
    let source = "\
#!/bin/bash
printf '%s\\n' \"${items[@]#$prefix/}\" \"${items[i]%$suffix}\"
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .indexed_array_reference_fragments()
                .iter()
                .map(|fragment| (fragment.span().slice(source), fragment.is_plain()))
                .collect::<Vec<_>>(),
            vec![
                ("${items[@]#$prefix/}", false),
                ("${items[i]%$suffix}", false),
            ]
        );
    });
}

#[test]
fn indexed_array_reference_fragments_record_subscript_index_behavior() {
    let source = "\
#!/bin/zsh
printf '%s\\n' ${arr[1]}
setopt ksh_arrays
printf '%s\\n' ${arr[1]}
if cond; then setopt ksh_zero_subscript; fi
printf '%s\\n' ${arr[1]}
unsetopt ksh_arrays
setopt ksh_zero_subscript
printf '%s\\n' ${arr[0]}
opt=ksh_zero_subscript
unsetopt \"$opt\"
printf '%s\\n' ${arr[0]}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .indexed_array_reference_fragments()
                .iter()
                .filter(|fragment| fragment.is_plain())
                .map(|fragment| {
                    (
                        fragment.span().slice(source),
                        fragment.subscript_index_behavior(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("${arr[1]}", SubscriptIndexBehavior::OneBased),
                ("${arr[1]}", SubscriptIndexBehavior::ZeroBased),
                ("${arr[1]}", SubscriptIndexBehavior::ZeroBased),
                ("${arr[0]}", SubscriptIndexBehavior::OneBasedWithZeroAlias,),
                ("${arr[0]}", SubscriptIndexBehavior::Ambiguous),
            ]
        );
    });
}

#[test]
fn indexed_array_reference_fragments_record_bash_subscript_index_behavior() {
    let source = "\
#!/bin/bash
printf '%s\\n' ${arr[0]}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .indexed_array_reference_fragments()
                .iter()
                .filter(|fragment| fragment.is_plain())
                .map(|fragment| {
                    (
                        fragment.span().slice(source),
                        fragment.subscript_index_behavior(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("${arr[0]}", SubscriptIndexBehavior::ZeroBased)]
        );
    });
}

#[test]
fn parameter_pattern_special_target_fragments_only_mark_direct_pattern_operands() {
    let source = "\
#!/bin/bash
scalar=${name#${items[0]}}
array_trim=\"${items[@]#$prefix/}\"
script_name=${0##*/}
nested=${items[i]%${name%$suffix}}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .parameter_pattern_special_target_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["$prefix", "${name%$suffix}"]
        );
    });
}

#[test]
fn positional_parameter_trim_fragments_cover_all_pattern_trim_forms() {
    let source = "\
#!/bin/sh
printf '%s\\n' \"${*%%dBm*}\" \"${*%dBm*}\" \"${*##dBm*}\" \"${*#dBm*}\"
printf '%s\\n' \"${@%%dBm*}\" \"${@%dBm*}\" \"${@##dBm*}\" \"${@#dBm*}\"
printf '%s\\n' \"${name%%dBm*}\"
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .positional_parameter_trim_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${*%%dBm*}",
                "${*%dBm*}",
                "${*##dBm*}",
                "${*#dBm*}",
                "${@%%dBm*}",
                "${@%dBm*}",
                "${@##dBm*}",
                "${@#dBm*}",
            ]
        );
    });
}

#[test]
fn backtick_fragments_remember_when_the_substitution_body_is_empty() {
    let source = "\
#!/bin/sh
echo \"Resolve the conflict and run ``${PROGRAM} --continue`` plus `date`.\"
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .backtick_fragments()
                .iter()
                .map(|fragment| (fragment.span().slice(source), fragment.is_empty()))
                .collect::<Vec<_>>(),
            vec![("``", true), ("``", true), ("`date`", false)]
        );
    });
}

#[test]
fn collects_declaration_assignment_probes_for_process_substitution_subscripts() {
    let source = "\
#!/bin/bash
\\declare -A arr[<(printf \"]\")]=$(date)
\\export out=<(printf hi)
";

    with_facts(source, None, |_, facts| {
        let probes = facts
            .structural_commands()
            .flat_map(|fact| fact.declaration_assignment_probes().iter())
            .map(|probe| (probe.target_name(), probe.has_command_substitution()))
            .collect::<Vec<_>>();

        assert_eq!(probes, vec![("arr", true), ("out", false)]);
    });
}

#[test]
fn ignores_readonly_like_tokens_after_escaped_declaration_assignments() {
    let source = "\
#!/bin/bash
demo() {
  \\declare out=$(date) -r
}
";

    with_facts(source, None, |_, facts| {
        let probes = facts
            .structural_commands()
            .flat_map(|fact| fact.declaration_assignment_probes().iter())
            .map(|probe| {
                (
                    probe.target_name(),
                    probe.readonly_flag(),
                    probe.has_command_substitution(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(probes, vec![("out", false, true)]);
    });
}
