use super::*;

#[test]
fn test_parse_assignment_value_after_line_continuation() {
    let input = "easyrsa_ksh=\\\n'value'\nprintf '%s\\n' \"$easyrsa_ksh\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let command = expect_simple(&script.body[0]);
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.name.render(input), "");
    assert!(command.args.is_empty());
    assert_eq!(command.assignments[0].target.name, "easyrsa_ksh");
}

#[test]
fn test_nested_expansion_in_array_subscript() {
    // ${arr[$RANDOM % ${#arr[@]}]} must parse without error.
    // The subscript contains ${#arr[@]} which has its own [ and ].
    let input = "echo ${arr[$RANDOM % ${#arr[@]}]}";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;
    assert_eq!(script.body.len(), 1);
    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(cmd.args.len(), 1);
        // The arg should contain an ArrayAccess with the full nested index
        let arg = &cmd.args[0];
        let has_array_access = arg.parts.iter().any(|p| {
            array_access_reference(&p.kind).is_some_and(|reference| {
                reference.name == "arr"
                    && reference
                        .subscript
                        .as_ref()
                        .is_some_and(|subscript| subscript.text.slice(input).contains("${#arr[@]}"))
            })
        });
        assert!(
            has_array_access,
            "expected ArrayAccess with nested index, got: {:?}",
            arg.parts
        );
    } else {
        panic!("expected simple command");
    }
}

/// Assignment with nested subscript must parse (previously caused fuel exhaustion).

#[test]
fn test_assignment_nested_subscript_parses() {
    let parser = Parser::new("x=${arr[$RANDOM % ${#arr[@]}]}");
    assert!(
        parser.parse().is_ok(),
        "assignment with nested subscript should parse"
    );
}

#[test]
fn test_indexed_assignment_with_spaces_in_subscript_parses() {
    let input = "a[1 + 2]=3\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "a");
    expect_subscript(&command.assignments[0].target, input, "1 + 2");
    assert!(command.name.render(input).is_empty());
}

#[test]
fn test_parenthesized_indexed_assignment_is_not_function_definition() {
    let input = "a[(1+2)*3]=9\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "a");
    expect_subscript(&command.assignments[0].target, input, "(1+2)*3");
    assert!(command.name.render(input).is_empty());
}

#[test]
fn test_assignment_index_ast_tracks_arithmetic_subscripts() {
    let input = "a[i + 1]=x\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let assignment = &command.assignments[0];
    let subscript_ast = assignment
        .target
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_ref());
    let expr = subscript_ast.expect("expected arithmetic subscript AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected additive subscript");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_variable(left.as_ref(), "i");
    expect_number(right.as_ref(), input, "1");
}

#[test]
fn test_decl_name_and_array_access_attach_arithmetic_index_asts() {
    let input = "declare foo[1+2]\necho ${arr[i+1]}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration command");
    };
    let DeclOperand::Name(name) = &command.operands[0] else {
        panic!("expected declaration name operand");
    };
    let subscript_ast = name
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_ref());
    let expr = subscript_ast.expect("expected declaration index AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected additive expression in declaration index");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_number(left.as_ref(), input, "1");
    expect_number(right.as_ref(), input, "2");

    let AstCommand::Simple(command) = &script.body[1].command else {
        panic!("expected simple command");
    };
    let reference = expect_array_access(&command.args[0]);
    expect_subscript(reference, input, "i+1");
    let expr = reference
        .subscript
        .as_ref()
        .and_then(|subscript| subscript.arithmetic_ast.as_ref())
        .expect("expected array access index AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected additive array index");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_variable(left.as_ref(), "i");
    expect_number(right.as_ref(), input, "1");
}

#[test]
fn test_substring_and_array_slice_attach_arithmetic_companion_asts() {
    let input = "echo ${s:i+1:len*2} ${arr[@]:i:j}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let (_, offset_ast, length_ast) = expect_substring_part(&command.args[0].parts[0].kind);
    let offset_ast = offset_ast.as_ref().expect("expected substring offset AST");
    let ArithmeticExpr::Binary { left, op, right } = &offset_ast.kind else {
        panic!("expected additive substring offset");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Add);
    expect_variable(left, "i");
    expect_number(right, input, "1");
    let length_ast = length_ast.as_ref().expect("expected substring length AST");
    let ArithmeticExpr::Binary {
        left: len_left,
        op: len_op,
        right: len_right,
    } = &length_ast.kind
    else {
        panic!("expected multiplicative substring length");
    };
    assert_eq!(*len_op, ArithmeticBinaryOp::Multiply);
    expect_variable(len_left, "len");
    expect_number(len_right, input, "2");

    let (_, offset_ast, length_ast) = expect_array_slice_part(&command.args[1].parts[0].kind);
    expect_variable(
        offset_ast
            .as_ref()
            .expect("expected array slice offset AST"),
        "i",
    );
    expect_variable(
        length_ast
            .as_ref()
            .expect("expected array slice length AST"),
        "j",
    );
}

#[test]
fn test_non_arithmetic_subscripts_leave_companion_ast_empty() {
    let input = "echo ${arr[@]} ${arr[*]} ${map[\"key\"]}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let reference = expect_array_access(&command.args[0]);
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected first array subscript");
    assert_eq!(subscript.selector(), Some(SubscriptSelector::At));
    assert!(subscript.arithmetic_ast.is_none());

    let reference = expect_array_access(&command.args[1]);
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected second array subscript");
    assert_eq!(subscript.selector(), Some(SubscriptSelector::Star));
    assert!(subscript.arithmetic_ast.is_none());

    let reference = expect_array_access(&command.args[2]);
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected third array subscript");
    assert_eq!(subscript.selector(), None);
    assert!(subscript.arithmetic_ast.is_none());
}

#[test]
fn test_compound_array_assignment_preserves_mixed_element_kinds() {
    let input = "arr=(one [two]=2 [three]+=3 four)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };

    assert_eq!(array.kind, ArrayKind::Contextual);
    assert_eq!(array.elements.len(), 4);

    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert_eq!(first.span.slice(input), "one");

    let ArrayElem::Keyed { key, value } = &array.elements[1] else {
        panic!("expected keyed element");
    };
    assert_eq!(key.text.slice(input), "two");
    assert_eq!(key.interpretation, SubscriptInterpretation::Contextual);
    assert_eq!(value.span.slice(input), "2");

    let ArrayElem::KeyedAppend { key, value } = &array.elements[2] else {
        panic!("expected keyed append element");
    };
    assert_eq!(key.text.slice(input), "three");
    assert_eq!(key.interpretation, SubscriptInterpretation::Contextual);
    assert_eq!(value.span.slice(input), "3");

    let ArrayElem::Sequential(last) = &array.elements[3] else {
        panic!("expected trailing sequential element");
    };
    assert_eq!(last.span.slice(input), "four");
}

#[test]
fn test_assignment_append_and_keyed_append_stay_distinct() {
    let input = "arr+=one\nassoc=(one [key]+=value)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(first) = &script.body[0].command else {
        panic!("expected first simple command");
    };
    assert!(first.assignments[0].append);

    let AstCommand::Simple(second) = &script.body[1].command else {
        panic!("expected second simple command");
    };
    assert!(!second.assignments[0].append);
    let AssignmentValue::Compound(array) = &second.assignments[0].value else {
        panic!("expected compound assignment");
    };
    assert!(matches!(array.elements[1], ArrayElem::KeyedAppend { .. }));
}

#[test]
fn test_assignment_target_mixed_subscript_and_compound_value_stay_structured() {
    let input = "assoc[\"$key\"-suffix]=(\"$value\" plain)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let assignment = &command.assignments[0];
    assert_eq!(assignment.target.name.as_str(), "assoc");
    let subscript = assignment
        .target
        .subscript
        .as_ref()
        .expect("expected target subscript");
    assert_eq!(subscript.text.slice(input), "\"$key\"-suffix");

    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound assignment value");
    };
    assert_eq!(array.elements.len(), 2);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert_eq!(first.span.slice(input), "\"$value\"");
    let ArrayElem::Sequential(second) = &array.elements[1] else {
        panic!("expected second sequential element");
    };
    assert_eq!(second.span.slice(input), "plain");
}

#[test]
fn test_compound_array_value_words_track_top_level_unquoted_commas() {
    let input = "\
arr=(
  alpha,beta
  head,$tail
  [k]=v,
  \"alpha,beta\"
  $'alpha,beta'
  $(printf %s 1,2)
  <(printf %s 1,2)
  >(printf %s 3,4)
  ${x/a,b/c}
  ${x/`echo }`/a,b}
  ${x/<(echo })/foo,bar}
  $((1,2))
  foo,{x,y},bar
)
";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound array assignment");
    };

    assert_eq!(array.elements.len(), 13, "{:#?}", array.elements);

    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert!(first.has_top_level_unquoted_comma());

    let ArrayElem::Sequential(second) = &array.elements[1] else {
        panic!("expected second sequential element");
    };
    assert!(second.has_top_level_unquoted_comma());

    let ArrayElem::Keyed { value, .. } = &array.elements[2] else {
        panic!("expected keyed element");
    };
    assert!(value.has_top_level_unquoted_comma());

    for (index, expected_span) in [
        (3usize, "\"alpha,beta\""),
        (4, "$'alpha,beta'"),
        (5, "$(printf %s 1,2)"),
        (6, "<(printf %s 1,2)"),
        (7, ">(printf %s 3,4)"),
        (8, "${x/a,b/c}"),
        (9, "${x/`echo }`/a,b}"),
        (10, "${x/<(echo })/foo,bar}"),
        (11, "$((1,2))"),
    ] {
        let ArrayElem::Sequential(value) = &array.elements[index] else {
            panic!("expected sequential element at index {index}");
        };
        assert_eq!(value.span.slice(input), expected_span);
        assert!(
            !value.has_top_level_unquoted_comma(),
            "unexpected comma flag for {}",
            value.span.slice(input)
        );
    }

    let ArrayElem::Sequential(last) = &array.elements[12] else {
        panic!("expected trailing sequential element");
    };
    assert_eq!(last.span.slice(input), "foo,{x,y},bar");
    assert!(last.has_top_level_unquoted_comma());
}

#[test]
fn test_compound_array_process_substitution_stays_typed_for_comma_detection() {
    let input = "arr=(<(printf %s 1,2) >(printf %s 3,4))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound array assignment");
    };

    for (index, is_input) in [(0usize, true), (1usize, false)] {
        let ArrayElem::Sequential(value) = &array.elements[index] else {
            panic!("expected sequential element at index {index}");
        };
        assert!(!value.has_top_level_unquoted_comma());
        assert!(matches!(
            &value.parts[0].kind,
            WordPart::ProcessSubstitution { is_input: actual, .. } if *actual == is_input
        ));
    }
}

#[test]
fn test_assignment_value_preserves_mixed_quoted_boundaries() {
    let input = "foo=\"$bar\"baz echo\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let slices = top_level_part_slices(word, input);

    assert!(!is_fully_quoted(word));
    assert_eq!(slices, vec!["\"$bar\"", "baz"]);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected quoted prefix");
    };
    assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
}

#[test]
fn test_assignment_value_stays_quoted_when_entire_value_is_quoted() {
    let input = "foo=\"$bar\"\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let slices = top_level_part_slices(word, input);

    assert!(is_fully_quoted(word));
    assert_eq!(slices, vec!["\"$bar\""]);
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected fully quoted value");
    };
    assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
}

#[test]
fn test_backtick_assignments_after_quoted_heredoc_preserve_each_substitution() {
    let input = "\
cat <<\\_ACEOF
Use these variables to override the choices made by `configure' or to help
it to find libraries and programs with nonstandard names/locations.
_ACEOF
ac_dir_suffix=/`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`
ac_top_builddir_sub=`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`
";
    let script = Parser::new(input).parse().unwrap().file;

    assert_eq!(script.body.len(), 3, "{:#?}", script.body);

    let AstCommand::Simple(first) = &script.body[1].command else {
        panic!("expected simple command, got {:#?}", script.body[1].command);
    };
    let AssignmentValue::Scalar(first_value) = &first.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    assert_eq!(
        first_value
            .parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect::<Vec<_>>(),
        vec!["/", "`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`"]
    );
    let WordPart::CommandSubstitution { syntax, body } = &first_value.parts[1].kind else {
        panic!("expected backtick substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);
    assert!(!body.is_empty());

    let AstCommand::Simple(second) = &script.body[2].command else {
        panic!("expected simple command, got {:#?}", script.body[2].command);
    };
    let AssignmentValue::Scalar(second_value) = &second.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    assert_eq!(
        second_value
            .parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect::<Vec<_>>(),
        vec!["`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`"]
    );
    let WordPart::CommandSubstitution { syntax, body } = &second_value.parts[0].kind else {
        panic!("expected backtick substitution");
    };
    assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);
    assert!(!body.is_empty());
}

#[test]
fn test_compound_array_keeps_quoted_pipelined_heredoc_substitution_as_one_element() {
    let input = r#"# shellcheck shell=bash
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
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[1].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Compound(array) = &command.assignments[0].value else {
        panic!("expected compound assignment");
    };

    assert_eq!(array.elements.len(), 4);
    let rendered = array
        .elements
        .iter()
        .map(|element| match element {
            ArrayElem::Sequential(word) => word.span.slice(input).to_owned(),
            _ => panic!("expected sequential array element"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec![
            "-X",
            "POST",
            "-d",
            "\"$(\n    cat <<-EOF | tr '\\n' ' '\n      {\n        \"query\": \"query {\n          repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {\n            refs(refPrefix: \\\"refs/tags/\\\")\n          }\n        }\"\n      }\nEOF\n  )\"",
        ]
    );
}

#[test]
fn test_word_part_spans_track_nested_array_expansions() {
    let input = "echo ${arr[$RANDOM % ${#arr[@]}]}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let word = &command.args[0];

    assert_eq!(word.parts.len(), 1);
    assert_eq!(
        word.part_span(0).unwrap().slice(input),
        "${arr[$RANDOM % ${#arr[@]}]}"
    );

    let reference = array_access_reference(&word.parts[0].kind).expect("expected array access");
    let subscript = reference.subscript.as_deref().expect("expected subscript");
    assert!(subscript.is_source_backed());
    assert_eq!(subscript.text.slice(input), "$RANDOM % ${#arr[@]}");
}

#[test]
fn test_array_target_parameter_operations_normalize_to_bourne_operations() {
    let input = "echo ${arr[0]//x/y} ${arr[@],,} ${arr[1]^^pattern}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let (replace_reference, replace_operator, _) =
        expect_parameter_operation_part(&command.args[0].parts[0].kind);
    expect_subscript(replace_reference, input, "0");
    assert!(matches!(replace_operator, ParameterOp::ReplaceAll { .. }));

    let (lower_reference, lower_operator, lower_operand) =
        expect_parameter_operation_part(&command.args[1].parts[0].kind);
    expect_subscript(lower_reference, input, "@");
    assert!(matches!(lower_operator, ParameterOp::LowerAll));
    assert!(lower_operand.is_none());

    let (upper_reference, upper_operator, upper_operand) =
        expect_parameter_operation_part(&command.args[2].parts[0].kind);
    expect_subscript(upper_reference, input, "1");
    assert!(matches!(upper_operator, ParameterOp::UpperAll));
    assert_eq!(
        upper_operand
            .expect("expected case-modification operand")
            .slice(input),
        "pattern"
    );
}

#[test]
fn test_assignment_replacement_expansion_span_keeps_escaped_backslashes() {
    let input = "crypt=${crypt//\\\\/\\\\\\\\}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };

    let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
    let ParameterOp::ReplaceAll {
        pattern,
        replacement,
        replacement_word_ast,
    } = operator
    else {
        panic!("expected replace-all operator");
    };
    assert_eq!(pattern.span.slice(input), "\\\\");
    assert_eq!(replacement.slice(input), "\\\\\\\\");
    assert_eq!(replacement_word_ast.span.slice(input), "\\\\\\\\");
    assert_eq!(
        top_level_part_slices(word, input),
        vec!["${crypt//\\\\/\\\\\\\\}"]
    );
}

#[test]
fn test_quoted_assignment_replacement_expansion_span_keeps_escaped_quotes() {
    let input = r#"query="${query//\"/\\\"}""#;
    assert_eq!(
        Parser::scan_array_parameter_expansion_len(r#"query//\"/\\\"}""#),
        Some(r#"query//\"/\\\"}"#.len())
    );
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double quoted value");
    };

    assert_eq!(parts[0].span.slice(input), r#"${query//\"/\\\"}"#);
    let (_, operator, _) = expect_parameter_operation_part(&parts[0].kind);
    assert!(matches!(operator, ParameterOp::ReplaceAll { .. }));
}

#[test]
fn test_quoted_assignment_replacement_expansion_span_keeps_escaped_slashes() {
    let input = r#"url_path="${url_path//https:\\/\\/api.openai.com\/v1}""#;
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        panic!("expected scalar assignment");
    };
    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double quoted value");
    };

    assert_eq!(
        parts[0].span.slice(input),
        r#"${url_path//https:\\/\\/api.openai.com\/v1}"#
    );
    let (_, operator, _) = expect_parameter_operation_part(&parts[0].kind);
    assert!(matches!(operator, ParameterOp::ReplaceAll { .. }));
}

#[test]
fn test_parse_arithmetic_command_distinguishes_assignment_from_comparison() {
    let input = "(( a = b == c ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Assignment { target, op, value } = &expr.kind else {
        panic!("expected arithmetic assignment");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable assignment target");
    };
    assert_eq!(name, "a");

    let ArithmeticExpr::Binary {
        left,
        op: cmp_op,
        right,
    } = &value.kind
    else {
        panic!("expected comparison on assignment right-hand side");
    };
    assert_eq!(*cmp_op, ArithmeticBinaryOp::Equal);
    expect_variable(left, "b");
    expect_variable(right, "c");
}

#[test]
fn test_parse_conditional_var_ref_operand_preserves_quoted_subscript_syntax() {
    let input = "[[ -v assoc[\"key\"] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };

    let subscript = expect_subscript_syntax(var_ref, input, "\"key\"", "key");
    assert!(matches!(subscript.kind, SubscriptKind::Ordinary));
}

#[test]
fn test_parse_conditional_var_ref_operand_preserves_spaced_zero_subscript() {
    let input = "[[ -v assoc[ 0 ] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };

    let subscript = expect_subscript(var_ref, input, " 0 ");
    assert!(matches!(
        subscript.arithmetic_ast.as_ref().map(|expr| &expr.kind),
        Some(ArithmeticExpr::Number(_))
    ));
}

#[test]
fn test_parse_conditional_var_ref_operand_preserves_nested_arithmetic_subscript() {
    let input = "[[ -v assoc[$((0))] ]]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Conditional(command) = compound else {
        panic!("expected conditional compound command");
    };

    let ConditionalExpr::Unary(unary) = &command.expression else {
        panic!("expected unary conditional");
    };
    let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
        panic!("expected typed var-ref operand");
    };

    let subscript = expect_subscript(var_ref, input, "$((0))");
    assert!(subscript.arithmetic_ast.is_some());
}

#[test]
fn test_parse_declare_clause_classifies_operands_and_prefix_assignments() {
    let input = "FOO=1 declare -a arr=(\"hello world\" two) bar >out\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "declare");
    assert_eq!(command.variant_span.slice(input), "declare");
    assert_eq!(command.assignments.len(), 1);
    assert_eq!(command.assignments[0].target.name, "FOO");
    assert_eq!(script.body[0].redirects.len(), 1);
    assert_eq!(
        redirect_word_target(&script.body[0].redirects[0])
            .span
            .slice(input),
        "out"
    );
    assert_eq!(command.operands.len(), 3);

    let DeclOperand::Flag(flag) = &command.operands[0] else {
        panic!("expected flag operand");
    };
    assert_eq!(flag.span.slice(input), "-a");

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    assert_eq!(assignment.target.name, "arr");
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.kind, ArrayKind::Indexed);
    assert_eq!(array.elements.len(), 2);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected first sequential element");
    };
    assert!(is_fully_quoted(first));
    assert_eq!(first.span.slice(input), "\"hello world\"");
    let ArrayElem::Sequential(second) = &array.elements[1] else {
        panic!("expected second sequential element");
    };
    assert_eq!(second.span.slice(input), "two");

    let DeclOperand::Name(name) = &command.operands[2] else {
        panic!("expected bare name operand");
    };
    assert_eq!(name.name, "bar");
}

#[test]
fn test_parse_declare_a_threads_associative_kind_into_compound_array() {
    let input = "declare -A assoc=(one [foo]=bar [bar]+=baz two)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };

    assert_eq!(array.kind, ArrayKind::Associative);
    assert_eq!(array.elements.len(), 4);
    assert!(matches!(array.elements[0], ArrayElem::Sequential(_)));

    let ArrayElem::Keyed { key, .. } = &array.elements[1] else {
        panic!("expected keyed element");
    };
    assert_eq!(key.text.slice(input), "foo");
    assert_eq!(key.interpretation, SubscriptInterpretation::Associative);

    let ArrayElem::KeyedAppend { key, .. } = &array.elements[2] else {
        panic!("expected keyed append element");
    };
    assert_eq!(key.text.slice(input), "bar");
    assert_eq!(key.interpretation, SubscriptInterpretation::Associative);

    assert!(matches!(array.elements[3], ArrayElem::Sequential(_)));
}

#[test]
fn test_parse_declare_array_preserves_quoted_command_substitution_elements() {
    let input = "f() {\n\tlocal -a graphql_request=(\n\t\t-X POST\n\t\t-d \"$(\n\t\t\tcat <<-EOF | tr '\\n' ' '\n\t\t\t\t{\"query\":\"field, direction\"}\n\t\t\tEOF\n\t\t)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 4, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[3] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_array_append_preserves_pipeline_command_substitution_span() {
    let input = "CANDIDATES+=(\"$(echo \"$line\" | cut -d' ' -f2-)\")\n";
    let script = Parser::new(input).parse().unwrap().file;

    let assignment = match &script.body[0].command {
        AstCommand::Simple(command) => &command.assignments[0],
        AstCommand::Decl(command) => match &command.operands[0] {
            DeclOperand::Assignment(assignment) => assignment,
            operand => panic!("expected assignment operand, got {operand:#?}"),
        },
        command => panic!("expected assignment command, got {command:#?}"),
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    let ArrayElem::Sequential(word) = &array.elements[0] else {
        panic!("expected sequential array element");
    };

    let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
        panic!("expected double-quoted array element");
    };
    let WordPart::CommandSubstitution { body, .. } = &parts[0].kind else {
        panic!("expected command substitution");
    };

    assert_eq!(
        parts[0].span.slice(input),
        "$(echo \"$line\" | cut -d' ' -f2-)"
    );
    assert!(matches!(&body[0].command, AstCommand::Binary(_)));
}

#[test]
fn test_parse_declare_array_preserves_separator_comment_in_quoted_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf '%s' x;# comment with ) and ,\n\t\tprintf '%s' y\n\t\t)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_piped_heredoc_without_spacing_in_command_substitution() {
    let input = "f() {\n\tlocal -a graphql_request=(\n\t\t\"$(\ncat <<EOF|tr '\\n' ' '\n{\"query\":\"field, direction\"}\nEOF\n\t\t)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_parameter_expansion_with_right_paren_in_command_substitution()
{
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf %s ${x//foo/)},1)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_plain_case_words_in_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t$(printf %s 1,2; echo case in)\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| matches!(
            &part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            }
        )),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_ansi_c_quotes_in_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t$(printf %s $'a\\'b'; printf %s 1,2)\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| matches!(
            &part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            }
        )),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_backticks_with_right_parens_in_command_substitution() {
    let input = "f() {\n\tlocal -a parts=(\n\t\t$(printf %s `echo foo)`; printf %s ok)\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| matches!(
            &part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            }
        )),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_backticks_inside_parameter_expansions_in_command_substitution()
 {
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf %s ${x/`echo }`/foo)},1)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_declare_array_preserves_process_substitutions_inside_parameter_expansions_in_command_substitution()
 {
    let input = "f() {\n\tlocal -a parts=(\n\t\t\"$(printf %s ${x/<(echo })/foo)},1)\"\n\t)\n}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Function(function) = &script.body[0].command else {
        panic!("expected function");
    };
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };
    assert!(redirects.is_empty());
    let AstCommand::Decl(command) = &body[0].command else {
        panic!("expected declaration, got {:#?}", body[0].command);
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand, got {:#?}", command.operands);
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1, "{:#?}", array.elements);

    let ArrayElem::Sequential(payload) = &array.elements[0] else {
        panic!("expected payload element");
    };
    assert!(
        payload.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if parts.iter().any(|part| matches!(
                        &part.kind,
                        WordPart::CommandSubstitution {
                            syntax: CommandSubstitutionSyntax::DollarParen,
                            ..
                        }
                    ))
            )
        }),
        "{:#?}",
        payload.parts
    );
}

#[test]
fn test_parse_parameter_expansion_preserves_quoted_associative_subscripts() {
    let input = "printf '%s\\n' ${assoc[\"key\"]} ${assoc['k']}\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected simple command");
    };

    let first = expect_array_access(&command.args[1]);
    let second = expect_array_access(&command.args[2]);

    let first_subscript = expect_subscript_syntax(first, input, "\"key\"", "key");
    assert!(matches!(first_subscript.kind, SubscriptKind::Ordinary));
    assert_eq!(
        first_subscript
            .word_ast()
            .expect("expected subscript word AST")
            .span
            .slice(input),
        "\"key\""
    );
    assert!(matches!(
        first_subscript
            .word_ast()
            .expect("expected subscript word AST")
            .parts
            .as_slice(),
        [WordPartNode {
            kind: WordPart::DoubleQuoted { .. },
            ..
        }]
    ));
    assert_eq!(command.args[1].render_syntax(input), "${assoc[\"key\"]}");

    let second_subscript = expect_subscript_syntax(second, input, "'k'", "k");
    assert!(matches!(second_subscript.kind, SubscriptKind::Ordinary));
    assert_eq!(
        second_subscript
            .word_ast()
            .expect("expected subscript word AST")
            .render_syntax(input),
        "'k'"
    );
    assert!(matches!(
        second_subscript
            .word_ast()
            .expect("expected subscript word AST")
            .parts
            .as_slice(),
        [WordPartNode {
            kind: WordPart::SingleQuoted { .. },
            ..
        }]
    ));
    assert_eq!(command.args[2].render_syntax(input), "${assoc['k']}");
}

#[test]
fn test_parse_declare_a_preserves_quoted_associative_keys() {
    let input = "declare -A assoc=([\"key\"]=bar ['alt']+=baz)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };

    let ArrayElem::Keyed { key, .. } = &array.elements[0] else {
        panic!("expected keyed element");
    };
    assert_eq!(key.text.slice(input), "key");
    assert_eq!(key.syntax_text(input), "\"key\"");

    let ArrayElem::KeyedAppend { key, .. } = &array.elements[1] else {
        panic!("expected keyed append element");
    };
    assert_eq!(key.text.slice(input), "alt");
    assert_eq!(key.syntax_text(input), "'alt'");
}

#[test]
fn test_parse_export_uses_dynamic_operand_for_invalid_assignment() {
    let script = Parser::new("export foo-bar=(one two)\n")
        .parse()
        .unwrap()
        .file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "export");
    assert_eq!(command.operands.len(), 1);
    let DeclOperand::Dynamic(word) = &command.operands[0] else {
        panic!("expected dynamic operand");
    };
    assert_eq!(
        word.span.slice("export foo-bar=(one two)\n"),
        "foo-bar=(one two)"
    );
}

#[test]
fn test_parse_typeset_clause_classifies_flags_and_assignments() {
    let input = "typeset -xr VAR=value other\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    assert_eq!(command.variant, "typeset");
    assert_eq!(command.variant_span.slice(input), "typeset");
    assert_eq!(command.operands.len(), 3);

    let DeclOperand::Flag(flag) = &command.operands[0] else {
        panic!("expected flag operand");
    };
    assert_eq!(flag.span.slice(input), "-xr");

    let DeclOperand::Assignment(assignment) = &command.operands[1] else {
        panic!("expected assignment operand");
    };
    assert_eq!(assignment.target.name, "VAR");
    assert!(
        matches!(&assignment.value, AssignmentValue::Scalar(value) if value.span.slice(input) == "value")
    );

    let DeclOperand::Name(name) = &command.operands[2] else {
        panic!("expected bare name operand");
    };
    assert_eq!(name.name, "other");
}

#[test]
fn test_parse_assignment_word_handles_process_substitution_subscript() {
    let input = "\\declare -A arr[<(printf \"]\")]=$(date)\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Simple(command) = &script.body[0].command else {
        panic!("expected escaped declaration to parse as simple command");
    };
    let words = command.args[1..].iter().collect::<Vec<_>>();
    let assignment = Parser::parse_assignment_word_group(
        input,
        &words,
        Some(ArrayKind::Associative),
        SubscriptInterpretation::Contextual,
    )
    .expect("expected assignment word");

    assert_eq!(assignment.target.name, "arr");
    assert_eq!(assignment.target.name_span.slice(input), "arr");
    assert!(assignment.target.subscript.is_some());
    let AssignmentValue::Scalar(value) = &assignment.value else {
        panic!("expected scalar value");
    };
    assert_eq!(value.span.slice(input), "$(date)");
}

#[test]
fn test_parse_declaration_name_operand_preserves_nested_arithmetic_subscript() {
    let input = "declare assoc[$((0))]\n";
    let script = Parser::new(input).parse().unwrap().file;

    let AstCommand::Decl(command) = &script.body[0].command else {
        panic!("expected declaration clause");
    };

    let DeclOperand::Name(name) = &command.operands[0] else {
        panic!("expected declaration name operand");
    };
    let subscript = expect_subscript(name, input, "$((0))");
    assert!(subscript.arithmetic_ast.is_some());
}
