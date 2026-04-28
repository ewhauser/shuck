use super::*;
use crate::facts::{
    collect_precise_function_return_guard_suppressions,
    collect_precise_function_return_guard_suppressions_in_seq,
    stmt_is_non_test_return_status_guard, stmt_is_unary_test_return_status_guard,
};
use shuck_ast::{Command, CompoundCommand};

#[test]
fn assignment_value_facts_ignore_line_continuation_backslashes_for_shell_quoting_literals() {
    let source = "#!/bin/bash\npackages=$foo\\\n$bar\nprintf '%s\\n' \"$packages\"\n";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::AssignmentValue)
            .next()
            .expect("assignment value fact should exist");

        assert!(!fact.contains_shell_quoting_literals());
    });
}

#[test]
fn assignment_value_facts_keep_single_quoted_backslash_newlines_for_shell_quoting_literals() {
    let source = "#!/bin/bash\npackages='foo\\\nbar'\nprintf '%s\\n' $packages\n";

    with_facts(source, None, |_, facts| {
        let fact = facts
            .expansion_word_facts(ExpansionContext::AssignmentValue)
            .next()
            .expect("assignment value fact should exist");

        assert!(fact.contains_shell_quoting_literals());
    });
}

#[test]
fn background_semicolon_facts_report_plain_semicolons() {
    let source = "#!/bin/bash\necho x &;\necho y & ;\n";

    with_facts(source, None, |_, facts| {
        let spans = facts.background_semicolon_spans();

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].slice(source), ";");
        assert_eq!(spans[0].start.line, 2);
        assert_eq!(spans[1].slice(source), ";");
        assert_eq!(spans[1].start.line, 3);
    });
}

#[test]
fn background_semicolon_facts_ignore_case_item_terminators() {
    let source = "\
#!/bin/bash
case ${1-} in
  break) printf '%s\\n' ok &;;
  spaced) printf '%s\\n' ok & ;;
  fallthrough) printf '%s\\n' ok & ;&
  continue) printf '%s\\n' ok & ;;&
esac
";

    with_facts(source, None, |_, facts| {
        assert!(facts.background_semicolon_spans().is_empty());
    });
}

#[test]
fn redundant_echo_space_facts_capture_diagnostic_and_edit_spans() {
    let source = "#!/bin/bash\necho foo    bar    baz\necho foo  bar\n";

    with_facts(source, None, |_, facts| {
        let facts = facts.redundant_echo_space_facts();

        assert_eq!(facts.len(), 1);
        assert_eq!(
            facts[0].diagnostic_span().slice(source),
            "echo foo    bar    baz"
        );
        assert_eq!(facts[0].space_spans().len(), 2);
        assert_eq!(facts[0].space_spans()[0].slice(source), "    ");
        assert_eq!(facts[0].space_spans()[1].slice(source), "    ");
    });
}

#[test]
fn commented_continuation_facts_ignore_plain_comment_only_lines() {
    let source = "#!/bin/sh\necho hello \\\n  #world\n  foo\n";

    with_facts(source, None, |_, facts| {
        assert!(facts.commented_continuation_comment_spans().is_empty());
    });
}

#[test]
fn commented_continuation_facts_anchor_at_comment_backslash() {
    let source = "#!/bin/sh\necho hello \\\n  #world \\\n  foo\n";

    with_facts(source, None, |_, facts| {
        let spans = facts.commented_continuation_comment_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start.line, 3);
        assert_eq!(spans[0].start.column, 11);
        assert_eq!(spans[0].start, spans[0].end);
        assert_eq!(
            &source[spans[0].start.offset - 1..spans[0].start.offset],
            "\\"
        );
    });
}

#[test]
fn builds_command_facts_for_wrapped_and_nested_commands() {
    let source = "#!/bin/bash\ncommand printf '%s\\n' \"$(echo hi)\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);

    let outer = facts
        .structural_commands()
        .find(|fact| fact.effective_name_is("printf"))
        .expect("expected structural printf fact");

    assert_eq!(facts.commands().len(), 2);
    assert_eq!(outer.literal_name(), Some("command"));
    assert_eq!(outer.effective_name(), Some("printf"));
    assert_eq!(outer.wrappers(), &[WrapperKind::Command]);
    assert!(!outer.is_nested_word_command());
    assert_eq!(
        outer
            .options()
            .printf()
            .and_then(|printf| printf.format_word)
            .map(|word| word.span.slice(source)),
        Some("'%s\\n'")
    );

    let nested = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("echo"))
        .expect("expected nested echo fact");
    assert!(nested.is_nested_word_command());
    let ids = facts
        .commands()
        .iter()
        .map(|fact| fact.id().index())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

#[test]

fn exposes_structural_commands_and_id_lookups() {
    let source = "#!/bin/bash\necho \"$(printf x)\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);

    let structural = facts
        .structural_commands()
        .map(|fact| fact.effective_or_literal_name().unwrap().to_owned())
        .collect::<Vec<_>>();
    let all = facts
        .commands()
        .iter()
        .map(|fact| fact.effective_or_literal_name().unwrap().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(structural, vec!["echo"]);
    assert_eq!(all, vec!["echo", "printf"]);

    let echo_id = facts
        .command_id_for_stmt(&output.file.body[0])
        .expect("expected command id for top-level stmt");
    assert_eq!(
        facts.command(echo_id).effective_or_literal_name(),
        Some("echo")
    );
    assert_eq!(
        facts.command_id_for_command(&output.file.body[0].command),
        Some(echo_id)
    );
}

#[test]
fn precomputes_innermost_command_ids_for_nested_offsets() {
    let source = "#!/bin/bash\necho \"$(printf '%s' \"$(uname)\")\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);

    let outer_id = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_or_literal_name() == Some("echo"))
        .map(|fact| fact.id())
        .expect("expected outer echo command");
    let middle_id = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_or_literal_name() == Some("printf"))
        .map(|fact| fact.id())
        .expect("expected nested printf command");
    let inner_id = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_or_literal_name() == Some("uname"))
        .map(|fact| fact.id())
        .expect("expected nested uname command");

    let command_ids_by_offset = super::build_innermost_command_ids_by_offset(
        facts.commands().raw(),
        vec![
            source.find("echo").expect("expected echo offset"),
            source.find("printf").expect("expected printf offset"),
            source.find("uname").expect("expected uname offset"),
        ],
    );

    assert_eq!(
        super::precomputed_command_id_for_offset(
            &command_ids_by_offset,
            source.find("echo").expect("expected echo offset"),
        ),
        Some(outer_id)
    );
    assert_eq!(
        super::precomputed_command_id_for_offset(
            &command_ids_by_offset,
            source.find("printf").expect("expected printf offset"),
        ),
        Some(middle_id)
    );
    assert_eq!(
        super::precomputed_command_id_for_offset(
            &command_ids_by_offset,
            source.find("uname").expect("expected uname offset"),
        ),
        Some(inner_id)
    );
}

#[test]
fn tracks_nested_commands_inside_if_and_elif_conditions() {
    let source = "\
#!/bin/bash
if \"$( [[ -f if_path ]] )\"; then
  :
elif \"$( [[ -f elif_path ]] )\"; then
  :
fi
";

    with_facts(source, None, |_, facts| {
        let if_nested = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ -f if_path ]]")
            .expect("expected nested if condition command");
        assert!(if_nested.scope_read_source_words().is_empty());
        assert!(facts.is_if_condition_command(if_nested.id()));
        assert!(!facts.is_elif_condition_command(if_nested.id()));

        let elif_nested = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ -f elif_path ]]")
            .expect("expected nested elif condition command");
        assert!(elif_nested.scope_read_source_words().is_empty());
        assert!(facts.is_elif_condition_command(elif_nested.id()));
    });
}

#[test]
fn recognizes_precise_function_return_guard_shapes() {
    let source = "\
#!/bin/bash
build_config() {
  helper || return $?
  [[ -n \"${flag:-}\" ]] || return $?
  export out=\"$(printf ok)\"
}
pkg_check() {
  [[ -f \"${base}/include/$1\" ]] || return $?
  case \"$mode\" in
    a) ext=a ;;
    *) ext=b ;;
  esac
  file=\"$(find \"${base}\" -name \"$2.$ext\" | head -n 1)\"
  [[ -n \"$file\" ]] || return $?
}
";

    let output = Parser::new(source).parse().unwrap();
    let Command::Function(build_config) = &output.file.body[0].command else {
        panic!("expected function");
    };
    let Command::Compound(CompoundCommand::BraceGroup(build_body)) = &build_config.body.command
    else {
        panic!("expected brace-group function body");
    };
    let Command::Function(pkg_check) = &output.file.body[1].command else {
        panic!("expected second function");
    };
    let Command::Compound(CompoundCommand::BraceGroup(pkg_body)) = &pkg_check.body.command else {
        panic!("expected brace-group function body");
    };

    assert!(stmt_is_non_test_return_status_guard(&build_body[0], source));
    assert!(stmt_is_unary_test_return_status_guard(
        &build_body[1],
        source
    ));
    assert!(stmt_is_unary_test_return_status_guard(&pkg_body[0], source));

    let mut body_spans = Vec::new();
    collect_precise_function_return_guard_suppressions_in_seq(
        build_body,
        source,
        &mut body_spans,
        true,
    );
    collect_precise_function_return_guard_suppressions_in_seq(
        pkg_body,
        source,
        &mut body_spans,
        true,
    );
    assert_eq!(
        body_spans
            .iter()
            .map(|span| span.start.line)
            .collect::<Vec<_>>(),
        vec![4, 8]
    );

    let mut spans = Vec::new();
    collect_precise_function_return_guard_suppressions(&output.file.body, source, &mut spans);

    assert_eq!(
        spans.iter().map(|span| span.start.line).collect::<Vec<_>>(),
        vec![4, 8]
    );
}

#[test]
fn includes_nested_jq_file_operands_in_writer_scope_reads() {
    let source = "#!/bin/bash\ncat <<<$(jq '.dns={}' \"$cfg\") >\"$cfg\"\n";

    with_facts(source, None, |_, facts| {
        let jq = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("jq"))
            .expect("expected nested jq command");
        assert_eq!(
            jq.file_operand_words()
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$cfg\""]
        );

        let cat = facts
            .structural_commands()
            .find(|fact| fact.effective_name_is("cat"))
            .expect("expected structural cat command");
        assert_eq!(
            cat.scope_read_source_words()
                .iter()
                .map(|fact| fact.word().span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(jq '.dns={}' \"$cfg\")", "\"$cfg\""]
        );
    });
}

#[test]
fn parses_jq_input_modes_into_file_operands() {
    let source = "\
#!/bin/bash
jq --args '$ARGS.positional[0]' \"$cfg\"
jq --jsonargs '$ARGS.positional[0]' \"$cfg\"
jq --indent 2 --args '$ARGS.positional[0]' \"$cfg\"
jq --rawfile cfg \"$cfg\" '.dns=$cfg'
jq --slurpfile cfg \"$cfg\" '.dns=$cfg'
jq --argfile cfg \"$cfg\" '.dns=$cfg'
jq -nc '.x=1' \"$cfg\"
jq -Lnewmods '.x=1' \"$cfg\"
";

    with_facts(source, None, |_, facts| {
        let jq_commands = facts
            .structural_commands()
            .filter(|fact| fact.effective_name_is("jq"))
            .collect::<Vec<_>>();

        assert_eq!(
            jq_commands
                .iter()
                .map(|command| {
                    command
                        .file_operand_words()
                        .iter()
                        .map(|word| word.span.slice(source))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            vec![
                Vec::<&str>::new(),
                Vec::<&str>::new(),
                Vec::<&str>::new(),
                vec!["\"$cfg\""],
                vec!["\"$cfg\""],
                vec!["\"$cfg\""],
                Vec::<&str>::new(),
                vec!["\"$cfg\""],
            ]
        );
    });
}

#[test]
fn tracks_nested_if_and_elif_conditions_inside_while_conditions() {
    let source = "\
#!/bin/bash
while if \"$( [[ -f if_path ]] )\"; then
  :
elif \"$( [[ -f elif_path ]] )\"; then
  :
fi; do
  :
done
";

    with_facts(source, None, |_, facts| {
        let if_nested = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ -f if_path ]]")
            .expect("expected nested if condition command");
        assert!(if_nested.scope_read_source_words().is_empty());
        assert!(facts.is_if_condition_command(if_nested.id()));
        assert!(!facts.is_elif_condition_command(if_nested.id()));

        let elif_nested = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ -f elif_path ]]")
            .expect("expected nested elif condition command");
        assert!(elif_nested.scope_read_source_words().is_empty());
        assert!(facts.is_elif_condition_command(elif_nested.id()));
    });
}

#[test]
fn tracks_nested_while_and_until_conditions_inside_if_and_elif_conditions() {
    let source = "\
#!/bin/bash
if while [[ -f if_path ]]; do
  :
done; then
  :
elif until [[ -f elif_path ]]; do
  :
done; then
  :
fi
";

    with_facts(source, None, |_, facts| {
        let if_nested = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ -f if_path ]]")
            .expect("expected nested while condition command");
        assert!(if_nested.scope_read_source_words().is_empty());
        assert!(facts.is_if_condition_command(if_nested.id()));
        assert!(!facts.is_elif_condition_command(if_nested.id()));

        let elif_nested = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ -f elif_path ]]")
            .expect("expected nested until condition command");
        assert!(elif_nested.scope_read_source_words().is_empty());
        assert!(facts.is_if_condition_command(elif_nested.id()));
        assert!(facts.is_elif_condition_command(elif_nested.id()));
    });
}

#[test]
fn preserves_condition_related_span_outputs() {
    let source = "\
#!/bin/bash
if [[ -f foo ]]; then
  echo $?
elif [[ -f bar ]]; then
  echo $?
elif [ $? -eq 1 ]; then
  :
fi
while test -f baz; do
  echo $?
done
if [[ -n $mode ]]; then
  case $mode in
    foo) tend $? ;;
  esac
fi
$(printf top)
! $(printf negated)
$(printf short-circuit) && echo ok
if $(printf one); then
  :
fi
if $(command -v printf) --version >/dev/null 2>&1; then
  :
fi
while $(printf two); do
  :
done
until $(command -v printf) --help >/dev/null 2>&1; do
  break
done
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .command_substitution_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(printf top)",
                "$(printf negated)",
                "$(printf short-circuit)",
                "$(printf one)",
                "$(printf two)"
            ]
        );
        assert_eq!(
            facts
                .condition_status_capture_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?", "$?", "$?"]
        );
    });
}

#[test]
fn collects_c107_status_checks_in_reportable_test_contexts() {
    let source = "\
#!/bin/bash
run
if [ $? -ne 0 ]; then :; fi
[ $? -ne 0 ]
run && [ $? -eq 0 ]
run || [ $? -ne 0 ]
if (( $? != 0 )); then :; fi
while [[ $? -ne 0 ]]; do break; done
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .dollar_question_after_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?", "$?", "$?", "$?"]
        );
    });
}

#[test]
fn collects_c056_status_reads_after_sequential_test_statements() {
    let source = "\
#!/bin/bash
[[ \"${second_line}\" == \"quz\" ]];
tend $?
[[ ${s0} == \"${s2}\" ]] &&
[[ ${s1} != *f* ]]
tend $?
[[ \"${later}\" == \"ok\" ]]
if [ -f foo ]; then :; fi
saved=$?
while [ -f foo ]; do break; done
again=$?
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .condition_status_capture_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    });
}

#[test]
fn keeps_c056_off_one_off_test_followups_without_later_test_blocks() {
    let source = "\
#!/bin/bash
[[ \"${second_line}\" == \"quz\" ]]
tend $?
nextcmd
";

    with_facts(source, None, |_, facts| {
        assert!(
            facts.condition_status_capture_spans().is_empty(),
            "expected no C056 spans for one-off sequential test followup"
        );
    });
}

#[test]
fn collects_c056_when_later_loop_conditions_continue_testing() {
    let source = "\
#!/bin/bash
[ -f first ]
tend $?
while [ -f loop_one ]; do
  :
done
[ -f second ]
tend $?
until [ -f loop_two ]; do
  break
done
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .condition_status_capture_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    });
}

#[test]
fn collects_c056_when_later_if_conditions_continue_testing() {
    let source = "\
#!/bin/bash
[ -f first ]
tend $?
if [ -f later_if ]; then
  :
elif [ -f later_elif ]; then
  :
fi
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .condition_status_capture_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?"]
        );
    });
}

#[test]
fn collects_c056_for_complex_status_blocks_but_skips_late_repeats() {
    let source = "\
#!/bin/bash
tbegin one
A=a B=b C=c
evar_push A B C
pu=$?
A=A B=B C=C
evar_pop 1
po1=$?
[[ ${A}${B}${C} == \"ABc\" ]]
po2=$?
evar_pop 2
po3=$?
var=$(bash -c 'echo ${VAR+set}')
[[ ${pu}${po1}${po2}${po3}${A}${B}${C} == \"0000abc\" ]]
tend $?

tbegin two
VAR=1
evar_push_set VAR 2
pu=$?
[[ ${VAR} == \"2\" ]]
po1=$?
evar_pop
po2=$?
[[ ${pu}${po1}${po2}${VAR} == \"0001\" ]]
tend $?
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .condition_status_capture_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?"]
        );
    });
}

#[test]
fn collects_c056_for_shellspec_style_followup_chains_when_sibling_groups_continue_testing() {
    let source = "\
#!/bin/bash
(
  [ \"$line\" = ok ]
  [ $? -eq 0 ] && no_problem || affect
)

(
  [ \"$other\" = ok ]
  :
)
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .condition_status_capture_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?"]
        );
    });
}

#[test]
fn keeps_c107_off_plain_function_entry_checks() {
    let source = "\
#!/bin/bash
check_status() {
  if [ $? -ne 0 ]; then :; fi
  [ $? -ne 0 ]
  run && [ $? -ne 0 ]
}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .dollar_question_after_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    });
}

#[test]
fn keeps_getopts_case_facts_when_loop_body_has_nested_command_content() {
    let source = "\
#!/bin/bash
while getopts 'ab' opt; do
  echo \"$(printf warmup)\"
  case \"$opt\" in
    a)
      ;;
    b)
      echo \"$(printf body)\"
      ;;
  esac
done
";

    with_facts(source, None, |_, facts| {
        let [case] = facts.getopts_cases() else {
            panic!("expected one getopts case fact");
        };

        assert_eq!(
            case.case_span().slice(source),
            "case \"$opt\" in\n    a)\n      ;;\n    b)\n      echo \"$(printf body)\"\n      ;;\n  esac"
        );
        assert_eq!(
            case.handled_case_labels()
                .iter()
                .map(|label| label.label())
                .collect::<Vec<_>>(),
            vec!['a', 'b']
        );
        assert!(case.missing_options().is_empty());
    });
}

#[test]
fn records_invalid_flag_handler_coverage_for_getopts_cases() {
    let source = "\
#!/bin/sh
while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
  esac
done

while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    \\?) : ;;
  esac
done

while getopts 'a' opt; do
  case \"$opt\" in
    a) : ;;
    *) : ;;
  esac
done

while getopts 'ab' opt; do
  case \"$opt\" in
    [ab]) : ;;
  esac
done

while getopts ':a' opt; do
  case \"$opt\" in
    a) : ;;
    :) : ;;
  esac
done
";

    with_facts(source, None, |_, facts| {
        let cases = facts.getopts_cases();
        assert_eq!(cases.len(), 5);
        assert!(cases[0].missing_invalid_flag_handler());
        assert!(!cases[1].missing_invalid_flag_handler());
        assert!(!cases[2].missing_invalid_flag_handler());
        assert!(cases[3].has_unknown_coverage());
        assert!(!cases[3].missing_invalid_flag_handler());
        assert!(cases[4].missing_invalid_flag_handler());
    });
}
