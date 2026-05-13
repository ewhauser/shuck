use super::*;

#[test]
fn test_non_zsh_dialect_parses_zsh_modifier_forms_as_zsh_parameters() {
    let input = "print ${(%):-%x} ${(f)mapfile[$WD_CONFIG]//$HOME/~}\n";
    let script = Parser::new(input).parse().unwrap().file;
    let command = expect_simple(&script.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(first.target, ZshExpansionTarget::Empty));
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::Defaulting {
            kind: ZshDefaultingOp::UseDefault,
            ref operand,
            colon_variant: true,
            ..
        }) if operand.slice(input) == "%x"
    ));
    let first_operation = first.operation.as_ref().expect("expected zsh operation");
    assert_eq!(
        first_operation
            .operand_word_ast()
            .expect("expected defaulting operand word")
            .render(input),
        "%x"
    );

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &second.target else {
        panic!("expected zsh reference target");
    };
    assert_eq!(reference.name.as_str(), "mapfile");
    let subscript = expect_subscript(reference, input, "$WD_CONFIG");
    assert_eq!(subscript.syntax_text(input), "$WD_CONFIG");
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(input) == "$HOME" && replacement.slice(input) == "~"
    ));
    let second_operation = second.operation.as_ref().expect("expected zsh operation");
    assert_eq!(
        second_operation
            .pattern_word_ast()
            .expect("expected replacement pattern word")
            .render(input),
        "$HOME"
    );
    assert_eq!(
        second_operation
            .replacement_word_ast()
            .expect("expected replacement word")
            .render(input),
        "~"
    );
}

#[test]
fn test_zsh_parameter_expansion_qualifier_inside_compound_array_stays_single_element() {
    let source = "files=( $dir/${~pats}(N) )\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let AstCommand::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };

    assert_eq!(command.assignments.len(), 1);
    let assignment = &command.assignments[0];
    let AssignmentValue::Compound(array) = &assignment.value else {
        panic!("expected compound array assignment");
    };
    assert_eq!(array.elements.len(), 1);
    let ArrayElem::Sequential(first) = &array.elements[0] else {
        panic!("expected sequential array element");
    };
    assert_eq!(first.span.slice(source), "$dir/${~pats}(N)");
}

#[test]
fn test_zsh_parameter_modifier_records_modifier_and_target() {
    let source = "print ${(m)foo} ${(%):-%x}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    assert_eq!(first.raw_body.slice(source), "(m)foo");
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        first
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['m']
    );
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected direct zsh reference target");
    };
    assert_eq!(reference.name.as_str(), "foo");
    assert!(first.operation.is_none());

    let second = expect_parameter(&command.args[1]);
    assert_eq!(second.raw_body.slice(source), "(%):-%x");
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        second
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['%']
    );
    assert!(matches!(second.target, ZshExpansionTarget::Empty));
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::Defaulting {
            kind: ZshDefaultingOp::UseDefault,
            ref operand,
            colon_variant: true,
            ..
        }) if operand.slice(source) == "%x"
    ));
    let defaulting = second
        .operation
        .as_ref()
        .expect("expected defaulting operation");
    assert_eq!(
        defaulting
            .operand_word_ast()
            .expect("expected defaulting operand word")
            .span
            .slice(source),
        "%x"
    );
}

#[test]
fn test_zsh_parameter_modifier_groups_split_flags_and_preserve_delimited_args() {
    let source = "print ${(Az)LBUFFER} ${(s./.)_p9k__cwd} ${(pj./.)parts[1,MATCH]}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        first
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['A', 'z']
    );
    assert!(
        first
            .modifiers
            .iter()
            .all(|modifier| modifier.span == first.modifiers[0].span)
    );
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "LBUFFER");

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        second
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['s']
    );
    assert_eq!(second.modifiers[0].argument_delimiter, Some('.'));
    assert_eq!(
        second.modifiers[0]
            .argument
            .as_ref()
            .expect("expected modifier argument")
            .slice(source),
        "/"
    );
    assert_eq!(
        second.modifiers[0]
            .argument_word_ast()
            .expect("expected modifier argument word")
            .span
            .slice(source),
        "/"
    );
    let ZshExpansionTarget::Reference(reference) = &second.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "_p9k__cwd");

    let third = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(third) = &third.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        third
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['p', 'j']
    );
    assert!(
        third
            .modifiers
            .iter()
            .all(|modifier| modifier.span == third.modifiers[0].span)
    );
    assert_eq!(third.modifiers[1].argument_delimiter, Some('.'));
    assert_eq!(
        third.modifiers[1]
            .argument
            .as_ref()
            .expect("expected modifier argument")
            .slice(source),
        "/"
    );
    assert_eq!(
        third.modifiers[1]
            .argument_word_ast()
            .expect("expected modifier argument word")
            .render(source),
        "/"
    );
    let ZshExpansionTarget::Reference(reference) = &third.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "parts");
    let subscript = expect_subscript(reference, source, "1,MATCH");
    assert_eq!(subscript.syntax_text(source), "1,MATCH");
}

#[test]
fn test_zsh_parameter_word_target_preserves_non_reference_target_text() {
    let source = "print ${^$(pidof zsh):#$$}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['^']
    );
    let ZshExpansionTarget::Word(word) = &parameter.target else {
        panic!("expected word target");
    };
    assert_eq!(word.render(source), "$(pidof zsh)");
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::PatternOperation {
            kind: ZshPatternOp::Filter,
            ref operand,
            ..
        }) if operand.slice(source) == "$$"
    ));
    let operation = parameter
        .operation
        .as_ref()
        .expect("expected pattern operation");
    assert_eq!(
        operation
            .operand_word_ast()
            .expect("expected pattern operand word")
            .render(source),
        "$$"
    );
}

#[test]
fn test_zsh_parameter_bare_prefix_flags_record_modifier_sequence() {
    let source = "print ${=name} ${~foo} ${^^bar} ${~~baz}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let split = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(split) = &split.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        split
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['=']
    );
    let ZshExpansionTarget::Reference(reference) = &split.target else {
        panic!("expected split target reference");
    };
    assert_eq!(reference.name.as_str(), "name");

    let glob = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(glob) = &glob.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        glob.modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['~']
    );
    let ZshExpansionTarget::Reference(reference) = &glob.target else {
        panic!("expected glob target reference");
    };
    assert_eq!(reference.name.as_str(), "foo");

    let rc_expand = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(rc_expand) = &rc_expand.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        rc_expand
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['^', '^']
    );
    let ZshExpansionTarget::Reference(reference) = &rc_expand.target else {
        panic!("expected rc-expand target reference");
    };
    assert_eq!(reference.name.as_str(), "bar");

    let glob_off = expect_parameter(&command.args[3]);
    let ParameterExpansionSyntax::Zsh(glob_off) = &glob_off.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        glob_off
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['~', '~']
    );
    let ZshExpansionTarget::Reference(reference) = &glob_off.target else {
        panic!("expected glob-off target reference");
    };
    assert_eq!(reference.name.as_str(), "baz");
}

#[test]
fn test_zsh_bare_dollar_prefix_flag_parses_as_parameter_word() {
    let source = "$=UNPACKCMD $=arr[1] $=+ice[extract]\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let parameter = expect_parameter(&command.name);
    assert_eq!(parameter.raw_body.slice(source), "=UNPACKCMD");
    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['=']
    );
    let ZshExpansionTarget::Reference(reference) = &parameter.target else {
        panic!("expected split target reference");
    };
    assert_eq!(reference.name.as_str(), "UNPACKCMD");

    let subscripted = expect_parameter(&command.args[0]);
    assert_eq!(subscripted.raw_body.slice(source), "=arr[1]");
    let ParameterExpansionSyntax::Zsh(subscripted) = &subscripted.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &subscripted.target else {
        panic!("expected subscripted target reference");
    };
    assert_eq!(reference.name.as_str(), "arr");
    expect_subscript(reference, source, "1");

    let probe = expect_parameter(&command.args[1]);
    assert_eq!(probe.raw_body.slice(source), "=+ice[extract]");
    let ParameterExpansionSyntax::Zsh(probe) = &probe.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &probe.target else {
        panic!("expected probe target reference");
    };
    assert_eq!(reference.name.as_str(), "+ice");
    expect_subscript(reference, source, "extract");
}

#[test]
fn test_zsh_parameter_word_target_accepts_quoted_command_substitution_text() {
    let source = "print ${\"$(xcode-select -p)\"%%/Contents/Developer*}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(parameter.length_prefix.is_none());
    let ZshExpansionTarget::Word(word) = &parameter.target else {
        panic!("expected quoted word target");
    };
    assert_eq!(word.span.slice(source), "\"$(xcode-select -p)\"");
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemoveSuffixLong,
            ref operand,
            ..
        }) if operand.slice(source) == "/Contents/Developer*"
    ));
    let operation = parameter
        .operation
        .as_ref()
        .expect("expected trim operation");
    assert_eq!(
        operation
            .operand_word_ast()
            .expect("expected trim operand word")
            .span
            .slice(source),
        "/Contents/Developer*"
    );
}

#[test]
fn test_zsh_parameter_length_prefix_preserves_nested_replacement_target() {
    let source = "print ${#${cd//${~q}/}}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .length_prefix
            .expect("expected zsh length prefix")
            .slice(source),
        "#"
    );
    let ZshExpansionTarget::Nested(inner) = &parameter.target else {
        panic!("expected nested zsh target");
    };
    assert_eq!(inner.raw_body.slice(source), "cd//${~q}/");
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected nested zsh syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected nested replacement target reference");
    };
    assert_eq!(reference.name.as_str(), "cd");
    assert!(matches!(
        inner.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "${~q}" && replacement.slice(source).is_empty()
    ));
    let operation = inner
        .operation
        .as_ref()
        .expect("expected replacement operation");
    assert_eq!(
        operation
            .pattern_word_ast()
            .expect("expected replacement pattern word")
            .span
            .slice(source),
        "${~q}"
    );
}

#[test]
fn test_zsh_parameter_colon_modifiers_preserve_targets_without_bourne_slice_offsets() {
    let source = "print ${REPLY:l} ${1:t} ${0:h}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let reply = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(reply) = &reply.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &reply.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "REPLY");
    assert!(matches!(
        reply.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":l"
    ));

    let first = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected positional reference target");
    };
    assert_eq!(reference.name.as_str(), "1");
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t"
    ));

    let zero = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(zero) = &zero.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &zero.target else {
        panic!("expected script-name reference target");
    };
    assert_eq!(reference.name.as_str(), "0");
    assert!(matches!(
        zero.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":h"
    ));
    let operation = zero.operation.as_ref().expect("expected unknown operation");
    assert_eq!(
        operation
            .operand_word_ast()
            .expect("expected unknown operation word")
            .render(source),
        ":h"
    );
}

#[test]
fn test_zsh_length_prefix_preserves_colon_modifier_targets() {
    let source = "print ${#link:t} ${#*:#0} ${#BUFFER:$highlight_start_index}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let link = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(link) = &link.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        link.length_prefix.expect("expected length").slice(source),
        "#"
    );
    let ZshExpansionTarget::Reference(reference) = &link.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "link");
    assert!(matches!(
        link.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t"
    ));

    let filtered_args = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(filtered_args) = &filtered_args.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &filtered_args.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "*");
    assert!(matches!(
        filtered_args.operation,
        Some(ZshExpansionOperation::PatternOperation {
            kind: ZshPatternOp::Filter,
            ref operand,
            ..
        }) if operand.slice(source) == "0"
    ));

    let sliced = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(sliced) = &sliced.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &sliced.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "BUFFER");
    assert!(matches!(
        sliced.operation,
        Some(ZshExpansionOperation::Slice {
            ref offset,
            length: None,
            ..
        }) if offset.slice(source) == "$highlight_start_index"
    ));
}

#[test]
fn test_zsh_parameter_colon_modifiers_with_digits_preserve_targets() {
    let source = "print ${path:A:h3} ${path:t2}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &first.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "path");
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":A:h3"
    ));

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &second.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "path");
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t2"
    ));
}

#[test]
fn test_zsh_plain_positional_parameters_preserve_bourne_references() {
    let source = "print ${1} ${10} ${#1} ${#10}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    assert!(matches!(
        &first.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "1"
    ));

    let second = expect_parameter(&command.args[1]);
    assert!(matches!(
        &second.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "10"
    ));

    let third = expect_parameter(&command.args[2]);
    assert!(matches!(
        &third.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "1"
    ));

    let fourth = expect_parameter(&command.args[3]);
    assert!(matches!(
        &fourth.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference })
            if reference.name.as_str() == "10"
    ));
}

#[test]
fn test_zsh_additional_upstream_parameter_examples_parse() {
    for source in [
        "echo ${${var}:u}\n",
        "echo ${var:-default}\n",
        "echo ${var:=default}\n",
        "echo ${var:-}\necho ${var:=}\n",
        "echo \"${REPLY%%$'\\n'}\"\n",
    ] {
        Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
    }

    let source = "echo ${array[(i)pattern]}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(source), "${array[(i)pattern]}");
}

#[test]
fn test_zsh_numeric_parameter_assignments_parse_as_assignments() {
    let source = "0=${(%):-%N}\n1=value\n2+=more\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();

    let targets = output
        .file
        .body
        .iter()
        .map(|stmt| {
            let command = expect_simple(stmt);
            assert_eq!(command.name.render(source), "");
            assert_eq!(command.assignments.len(), 1);
            command.assignments[0].target.name.as_str()
        })
        .collect::<Vec<_>>();

    assert_eq!(targets, vec!["0", "1", "2"]);
    assert!(!expect_simple(&output.file.body[0]).assignments[0].append);
    assert!(expect_simple(&output.file.body[2]).assignments[0].append);
}

#[test]
fn test_zsh_parameter_identifier_slices_preserve_legacy_slice_parts() {
    let source = "print ${foo:i} ${foo:i:j}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let (first_reference, first_offset_ast, first_length_ast) =
        expect_substring_part(&command.args[0].parts[0].kind);
    assert_eq!(first_reference.name.as_str(), "foo");
    assert_eq!(
        first_offset_ast
            .as_ref()
            .expect("expected first offset AST")
            .span
            .slice(source),
        "i"
    );
    assert!(first_length_ast.is_none());

    let (second_reference, second_offset_ast, second_length_ast) =
        expect_substring_part(&command.args[1].parts[0].kind);
    assert_eq!(second_reference.name.as_str(), "foo");
    assert_eq!(
        second_offset_ast
            .as_ref()
            .expect("expected second offset AST")
            .span
            .slice(source),
        "i"
    );
    assert_eq!(
        second_length_ast
            .as_ref()
            .expect("expected second length AST")
            .span
            .slice(source),
        "j"
    );
}

#[test]
fn test_zsh_parameter_identifier_slices_stay_typed_in_zsh_parameter_nodes() {
    let source = "print ${(m)foo:i:j}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let parameter = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['m']
    );
    let ZshExpansionTarget::Reference(reference) = &parameter.target else {
        panic!("expected reference target");
    };
    assert_eq!(reference.name.as_str(), "foo");
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::Slice {
            ref offset,
            length: Some(ref length),
            ..
        }) if offset.slice(source) == "i" && length.slice(source) == "j"
    ));
    let operation = parameter
        .operation
        .as_ref()
        .expect("expected slice operation");
    assert_eq!(
        operation
            .offset_word_ast()
            .expect("expected slice offset word")
            .render(source),
        "i"
    );
    assert_eq!(
        operation
            .length_word_ast()
            .expect("expected slice length word")
            .render(source),
        "j"
    );
}

#[test]
fn test_zsh_nested_parameter_modifier_records_nested_target_and_pattern_operation() {
    let source = "print ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);
    let parameter = expect_parameter(&command.args[0]);

    let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert_eq!(
        parameter
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['M']
    );
    let ZshExpansionTarget::Nested(inner) = &parameter.target else {
        panic!("expected nested zsh parameter target");
    };
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected nested zsh syntax");
    };
    assert_eq!(
        inner
            .modifiers
            .iter()
            .map(|modifier| modifier.name)
            .collect::<Vec<_>>(),
        vec!['k']
    );
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected nested reference target");
    };
    assert_eq!(reference.name.as_str(), "parameters");
    assert!(reference.has_array_selector());
    assert!(matches!(
        parameter.operation,
        Some(ZshExpansionOperation::PatternOperation {
            kind: ZshPatternOp::Filter,
            ref operand,
            ..
        }) if operand.slice(source) == "__gitcomp_builtin_*"
    ));
}

#[test]
fn test_zsh_nested_plain_access_targets_preserve_bourne_refs_without_modifier_regression() {
    let source = "print ${${10}} ${#${10}} ${${#}} ${${$}} ${${1:t}}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let nested = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(nested) = &nested.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &nested.target else {
        panic!("expected nested target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "10"
    ));

    let length = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(length) = &length.syntax else {
        panic!("expected outer zsh syntax");
    };
    assert_eq!(
        length
            .length_prefix
            .expect("expected zsh length prefix")
            .slice(source),
        "#"
    );
    let ZshExpansionTarget::Nested(inner) = &length.target else {
        panic!("expected nested length target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "10"
    ));

    let count = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(count) = &count.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &count.target else {
        panic!("expected nested count target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "#"
    ));

    let pid = expect_parameter(&command.args[3]);
    let ParameterExpansionSyntax::Zsh(pid) = &pid.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &pid.target else {
        panic!("expected nested pid target");
    };
    assert!(matches!(
        &inner.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "$"
    ));

    let modifier = expect_parameter(&command.args[4]);
    let ParameterExpansionSyntax::Zsh(modifier) = &modifier.syntax else {
        panic!("expected outer zsh syntax");
    };
    let ZshExpansionTarget::Nested(inner) = &modifier.target else {
        panic!("expected nested modifier target");
    };
    let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
        panic!("expected inner zsh syntax");
    };
    let ZshExpansionTarget::Reference(reference) = &inner.target else {
        panic!("expected positional reference target");
    };
    assert_eq!(reference.name.as_str(), "1");
    assert!(matches!(
        inner.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. }) if text.slice(source) == ":t"
    ));
}

#[test]
fn test_zsh_parameter_supported_operations_are_typed_and_preserve_source_spans() {
    let source = "print ${(m)foo#${needle}} ${(S)foo//\"pre\"$suffix/$replacement} ${(m)foo:$offset:${length}} ${(m)foo:^other}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let trim = expect_parameter(&command.args[0]);
    assert_eq!(trim.raw_body.slice(source), "(m)foo#${needle}");
    let ParameterExpansionSyntax::Zsh(trim) = &trim.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        trim.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemovePrefixShort,
            ref operand,
            ..
        }) if operand.is_source_backed() && operand.slice(source) == "${needle}"
    ));
    let trim_operation = trim.operation.as_ref().expect("expected trim operation");
    assert_eq!(
        trim_operation
            .operand_word_ast()
            .expect("expected trim operand word")
            .span
            .slice(source),
        "${needle}"
    );

    let replacement = expect_parameter(&command.args[1]);
    assert_eq!(
        replacement.raw_body.slice(source),
        "(S)foo//\"pre\"$suffix/$replacement"
    );
    let ParameterExpansionSyntax::Zsh(replacement) = &replacement.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        replacement.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceAll,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.is_source_backed()
            && pattern.slice(source) == "\"pre\"$suffix"
            && replacement.is_source_backed()
            && replacement.slice(source) == "$replacement"
    ));
    let replacement_operation = replacement
        .operation
        .as_ref()
        .expect("expected replacement operation");
    assert_eq!(
        replacement_operation
            .pattern_word_ast()
            .expect("expected replacement pattern word")
            .span
            .slice(source),
        "\"pre\"$suffix"
    );
    assert_eq!(
        replacement_operation
            .replacement_word_ast()
            .expect("expected replacement word")
            .span
            .slice(source),
        "$replacement"
    );

    let slice = expect_parameter(&command.args[2]);
    assert_eq!(slice.raw_body.slice(source), "(m)foo:$offset:${length}");
    let ParameterExpansionSyntax::Zsh(slice) = &slice.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        slice.operation,
        Some(ZshExpansionOperation::Slice {
            ref offset,
            length: Some(ref length),
            ..
        }) if offset.is_source_backed()
            && offset.slice(source) == "$offset"
            && length.is_source_backed()
            && length.slice(source) == "${length}"
    ));
    let slice_operation = slice.operation.as_ref().expect("expected slice operation");
    assert_eq!(
        slice_operation
            .offset_word_ast()
            .expect("expected slice offset word")
            .span
            .slice(source),
        "$offset"
    );
    assert_eq!(
        slice_operation
            .length_word_ast()
            .expect("expected slice length word")
            .span
            .slice(source),
        "${length}"
    );

    let unknown = expect_parameter(&command.args[3]);
    assert_eq!(unknown.raw_body.slice(source), "(m)foo:^other");
    let ParameterExpansionSyntax::Zsh(unknown) = &unknown.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        unknown.operation,
        Some(ZshExpansionOperation::Unknown { ref text, .. })
            if text.is_source_backed() && text.slice(source) == ":^other"
    ));
    let unknown_operation = unknown
        .operation
        .as_ref()
        .expect("expected unknown operation");
    assert_eq!(
        unknown_operation
            .operand_word_ast()
            .expect("expected unknown operation word")
            .span
            .slice(source),
        ":^other"
    );
}

#[test]
fn test_zsh_parameter_operation_kinds_cover_long_trim_and_anchored_replacement() {
    let source = "print ${(m)foo##pre*} ${(m)foo%%post*} ${(S)foo/#$prefix/$replacement} ${(S)foo/%$suffix/$replacement}\n";
    let output = Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let command = expect_simple(&output.file.body[0]);

    let first = expect_parameter(&command.args[0]);
    let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        first.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemovePrefixLong,
            ref operand,
            ..
        }) if operand.slice(source) == "pre*"
    ));

    let second = expect_parameter(&command.args[1]);
    let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        second.operation,
        Some(ZshExpansionOperation::TrimOperation {
            kind: ZshTrimOp::RemoveSuffixLong,
            ref operand,
            ..
        }) if operand.slice(source) == "post*"
    ));

    let third = expect_parameter(&command.args[2]);
    let ParameterExpansionSyntax::Zsh(third) = &third.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        third.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplacePrefix,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "$prefix" && replacement.slice(source) == "$replacement"
    ));

    let fourth = expect_parameter(&command.args[3]);
    let ParameterExpansionSyntax::Zsh(fourth) = &fourth.syntax else {
        panic!("expected zsh parameter syntax");
    };
    assert!(matches!(
        fourth.operation,
        Some(ZshExpansionOperation::ReplacementOperation {
            kind: ZshReplacementOp::ReplaceSuffix,
            ref pattern,
            replacement: Some(ref replacement),
            ..
        }) if pattern.slice(source) == "$suffix" && replacement.slice(source) == "$replacement"
    ));
}

#[test]
fn test_parse_zsh_parameter_default_with_prompt_escape_text() {
    let source = "color_green=${BATTERY_COLOR_GREEN:-%F{green}}\n";
    Parser::with_dialect(source, ShellDialect::Zsh)
        .parse()
        .unwrap();
}
