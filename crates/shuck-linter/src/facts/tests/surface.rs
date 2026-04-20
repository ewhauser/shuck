use super::*;

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
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]}", "${arr[@]}", "${arr[*]}"]
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
fn open_double_quote_surface_facts_ignore_empty_prefix_multiline_quotes_with_literal_suffix() {
    let source = "\
#!/bin/sh
echo \"\"\"line one
line two\"suffix
";

    with_facts(source, None, |_, facts| {
        assert!(facts.open_double_quote_fragments().is_empty());
        assert!(facts.suspect_closing_quote_fragments().is_empty());
    });
}

#[test]
fn open_double_quote_surface_facts_ignore_valid_multiline_quotes_with_suffix_expansion() {
    let source = "\
#!/bin/sh
echo \"line one
line two\"$suffix
";

    with_facts(source, None, |_, facts| {
        assert!(facts.open_double_quote_fragments().is_empty());
        assert!(facts.suspect_closing_quote_fragments().is_empty());
    });
}

#[test]
fn open_double_quote_surface_facts_ignore_empty_prefix_multiline_quotes_with_suffix_expansion() {
    let source = "\
#!/bin/sh
echo \"\"\"line one
line two\"$suffix
";

    with_facts(source, None, |_, facts| {
        assert!(facts.open_double_quote_fragments().is_empty());
        assert!(facts.suspect_closing_quote_fragments().is_empty());
    });
}

#[test]
fn open_double_quote_surface_facts_report_only_first_fragment_per_word() {
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

        assert_eq!(open, vec![(2, 6)]);
        assert_eq!(close, vec![(3, 5)]);
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
fn marks_subscript_index_references_without_span_scanning() {
    let source = "#!/bin/bash\nprintf '%s\\n' \"${arr[$idx]}\" \"$free\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let idx_reference = semantic
        .references()
        .iter()
        .find(|reference| reference.name.as_str() == "idx")
        .expect("expected idx reference");
    let free_reference = semantic
        .references()
        .iter()
        .find(|reference| reference.name.as_str() == "free")
        .expect("expected free reference");

    assert!(facts.is_subscript_index_reference(idx_reference.span));
    assert!(!facts.is_subscript_index_reference(free_reference.span));
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
fn builds_word_facts_with_contexts_hosts_and_anchor_spans() {
    let source = "\
#!/bin/bash
case literal in
  @($pat|$(printf '%s' case))) : ;;
esac
trap \"echo $x $(date)\" EXIT
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
fn builds_case_pattern_expansion_spans_for_mixed_and_quoted_patterns() {
    let source = "\
#!/bin/sh
case $value in
  x$pat) : ;;
  \"$quoted\") : ;;
  \"$left\"$right) : ;;
  x$left@(foo|bar)) : ;;
  @($nested|\"$ignored\")) : ;;
esac
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .case_pattern_expansion_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["x$pat", "\"$left\"$right", "x$left@(foo|bar)", "$nested"]
        );
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
    });
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
printf '%s\\n' \"foo\"bar\"baz\" \"foo\"-\"bar\" \"foo\"$(printf '%s' x)\"bar\" \"$left\"-\"$right\" x=\"$(cmd \"a\".\"b\")\" '$('\"foo\"parenmid\"baz\" '${'\"foo\"bracemid\"baz\"
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

        assert_eq!(spans, vec!["bar", "-", "parenmid", "bracemid", "."]);
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
alias gtl='gtl(){ git tag --sort=-v:refname -n -l \"${1}*\" }; noglob gtl'
alias hello='function hello { echo hi; }'
alias positional='${1+\"$@\"}'
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
                "gtl='gtl(){ git tag --sort=-v:refname -n -l \"${1}*\" }; noglob gtl'",
                "hello='function hello { echo hi; }'",
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
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${items[@]#$prefix/}", "${items[i]%$suffix}"]
        );
    });
}

#[test]
fn parameter_pattern_special_target_fragments_only_mark_host_expansions() {
    let source = "\
#!/bin/bash
scalar=${name#${items[0]}}
array_trim=\"${items[@]#$prefix/}\"
script_name=${0##*/}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .parameter_pattern_special_target_fragments()
                .iter()
                .map(|fragment| fragment.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["${items[@]#$prefix/}", "${0##*/}"]
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
";

    with_facts(source, None, |_, facts| {
        let probes = facts
            .structural_commands()
            .flat_map(|fact| fact.declaration_assignment_probes().iter())
            .map(|probe| (probe.target_name(), probe.has_command_substitution()))
            .collect::<Vec<_>>();

        assert_eq!(probes, vec![("arr", true)]);
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

#[test]
fn parses_assignment_words_with_process_substitution_subscripts() {
    let word = "arr[<(printf \"]\")]=$(date)";
    let parsed = super::parse_assignment_word(word)
        .map(|parsed| (parsed.name, &word[parsed.value_offset..]));

    assert_eq!(parsed, Some(("arr", "$(date)")));
}
