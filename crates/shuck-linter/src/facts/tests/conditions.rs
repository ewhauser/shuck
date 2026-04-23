use super::*;

#[test]
fn tracks_multi_statement_substitution_bodies() {
    let source = "\
#!/bin/sh
single=$(printf '%s\\n' ok)
multiple=$(printf '%s\\n' one; printf '%s\\n' two)
conditional=$( [[ -n $value ]] && printf '%s\\n' ok )
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.body_has_multiple_statements(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(substitutions.get("$(printf '%s\\n' ok)"), Some(&false));
        assert_eq!(
            substitutions.get("$(printf '%s\\n' one; printf '%s\\n' two)"),
            Some(&true)
        );
        assert_eq!(
            substitutions.get("$( [[ -n $value ]] && printf '%s\\n' ok )"),
            Some(&false)
        );
    });
}

#[test]
fn identifies_command_substitutions_that_echo_plain_text_or_expansions() {
    let source = "\
#!/bin/sh
plain=$(echo foo)
expanded=$(echo $foo)
quoted=$(echo \"$foo\")
var_suffix=$(echo foo$foo)
command_subst=$(echo foo $(date))
nested_only=$(echo $(basename \"$f\" .fuzz))
quoted_nested_only=$(echo \"$(basename \"$f\" .fuzz)\")
multiple_nested=$(echo $(date) $(pwd))
dynamic_dash=$(echo ${foo}-bar)
quoted_dynamic_dash=$(echo \"${foo}-bar\")
command_subst_dash=$(echo $(date)-bar)
pipeline_cut=$(echo \"$line\" | cut -d' ' -f2-)
option_like=$(echo -en \"\\001\")
glob_like=$(echo O*)
brace_like=$(echo {a,b})
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.body_contains_echo(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(substitutions.get("$(echo foo)"), Some(&true));
        assert_eq!(substitutions.get("$(echo $foo)"), Some(&true));
        assert_eq!(substitutions.get("$(echo \"$foo\")"), Some(&true));
        assert_eq!(substitutions.get("$(echo foo$foo)"), Some(&true));
        assert_eq!(substitutions.get("$(echo foo $(date))"), Some(&true));
        assert_eq!(
            substitutions.get("$(echo $(basename \"$f\" .fuzz))"),
            Some(&false)
        );
        assert_eq!(
            substitutions.get("$(echo \"$(basename \"$f\" .fuzz)\")"),
            Some(&false)
        );
        assert_eq!(substitutions.get("$(echo $(date) $(pwd))"), Some(&true));
        assert_eq!(substitutions.get("$(echo ${foo}-bar)"), Some(&false));
        assert_eq!(substitutions.get("$(echo \"${foo}-bar\")"), Some(&false));
        assert_eq!(substitutions.get("$(echo $(date)-bar)"), Some(&false));
        assert_eq!(
            substitutions.get("$(echo \"$line\" | cut -d' ' -f2-)"),
            Some(&false)
        );
        assert_eq!(substitutions.get("$(echo -en \"\\001\")"), Some(&false));
        assert_eq!(substitutions.get("$(echo O*)"), Some(&false));
        assert_eq!(substitutions.get("$(echo {a,b})"), Some(&false));
    });
}

#[test]
fn keeps_bash_pipeline_substitution_facts_false_in_pattern_and_array_contexts() {
    let source = "\
#!/bin/bash
if [[ \"${currencyCodes[*]}\" == *\"$(echo \"${@}\" | tr -d '[:space:]')\"* ]]; then :; fi
CANDIDATES+=(\"$(echo \"$line\" | cut -d' ' -f2-)\")
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .filter(|fact| fact.span().slice(source).contains("$(echo"))
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.body_contains_echo(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            substitutions,
            vec![
                ("$(echo \"${@}\" | tr -d '[:space:]')".to_owned(), false,),
                ("$(echo \"$line\" | cut -d' ' -f2-)".to_owned(), false),
            ]
        );
    });
}

#[test]
fn identifies_command_substitutions_that_grep_output_directly() {
    let source = "\
#!/bin/sh
plain=$(grep foo input.txt)
quiet=$(grep -q foo input.txt)
egrep_plain=$(egrep foo input.txt)
fgrep_plain=$(fgrep foo input.txt)
nested_pipeline=$(echo foo | grep foo input.txt)
escaped_pipeline=$(echo foo | \\grep foo input.txt)
nested=$(foo $(grep foo input.txt))
mixed=$(grep foo input.txt)$(date)
pipeline=$(grep foo input.txt | wc -l)
sequence=$(foo; grep foo input.txt)
and_chain=$(foo && grep foo input.txt)
legacy=`nvm ls | grep '^ *\\.'`
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.body_contains_grep(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(substitutions.get("$(grep foo input.txt)"), Some(&true));
        assert_eq!(substitutions.get("$(grep -q foo input.txt)"), Some(&true));
        assert_eq!(substitutions.get("$(egrep foo input.txt)"), Some(&true));
        assert_eq!(substitutions.get("$(fgrep foo input.txt)"), Some(&true));
        assert_eq!(
            substitutions.get("$(echo foo | grep foo input.txt)"),
            Some(&true)
        );
        assert_eq!(
            substitutions.get("$(echo foo | \\grep foo input.txt)"),
            Some(&true)
        );
        assert_eq!(
            substitutions.get("$(foo $(grep foo input.txt))"),
            Some(&false)
        );
        assert_eq!(
            substitutions.get("$(grep foo input.txt | wc -l)"),
            Some(&false)
        );
        assert_eq!(
            substitutions.get("$(foo; grep foo input.txt)"),
            Some(&false)
        );
        assert_eq!(
            substitutions.get("$(foo && grep foo input.txt)"),
            Some(&false)
        );
        assert_eq!(substitutions.get("`nvm ls | grep '^ *\\.'`"), Some(&true));
    });
}

#[test]
fn marks_redirect_only_input_command_substitutions_as_bash_file_slurps() {
    let source = "\
#!/bin/bash
printf '%s\\n' $(<input.txt) \"$( < spaced.txt )\" $(0< fd.txt) $(< quiet.txt 2>/dev/null) $(< muted.txt >/dev/null) $(< closed.txt 0<&-) $(cat < portable.txt) $(> out.txt) $(foo=bar)
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.is_bash_file_slurp(),
                )
            })
            .collect::<Vec<_>>();

        assert!(substitutions.contains(&("$(<input.txt)".to_owned(), true)));
        assert!(
            substitutions.contains(&("\"$( < spaced.txt )\"".trim_matches('"').to_owned(), true))
        );
        assert!(substitutions.contains(&("$(0< fd.txt)".to_owned(), true)));
        assert!(substitutions.contains(&("$(< quiet.txt 2>/dev/null)".to_owned(), false)));
        assert!(substitutions.contains(&("$(< muted.txt >/dev/null)".to_owned(), false)));
        assert!(substitutions.contains(&("$(< closed.txt 0<&-)".to_owned(), false)));
        assert!(substitutions.contains(&("$(cat < portable.txt)".to_owned(), false)));
        assert!(substitutions.contains(&("$(> out.txt)".to_owned(), false)));
        assert!(substitutions.contains(&("$(foo=bar)".to_owned(), false)));
    });
}

#[test]
fn builds_simple_test_facts_with_shapes_and_closing_bracket_validation() {
    let source = "\
#!/bin/sh
test
[ foo ]
[ -n foo ]
[ left = right ]
[ ! = right ]
[ ! -n foo ]
[ ! left = right ]
[ foo -eq 1 ]
[ missing
";

    with_facts(source, None, |_, facts| {
        let commands = facts
            .structural_commands()
            .map(|fact| (fact.span().slice(source).trim_end().to_owned(), fact))
            .collect::<Vec<_>>();

        let empty = commands
            .iter()
            .find(|(text, _)| text == "test")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected test fact");
        assert_eq!(empty.syntax(), SimpleTestSyntax::Test);
        assert_eq!(empty.shape(), SimpleTestShape::Empty);

        let truthy = commands
            .iter()
            .find(|(text, _)| text == "[ foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected truthy test fact");
        assert_eq!(truthy.syntax(), SimpleTestSyntax::Bracket);
        assert_eq!(truthy.shape(), SimpleTestShape::Truthy);
        assert!(
            truthy
                .truthy_operand_class()
                .is_some_and(|class| class.is_fixed_literal())
        );

        let unary = commands
            .iter()
            .find(|(text, _)| text == "[ -n foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected unary test fact");
        assert_eq!(unary.shape(), SimpleTestShape::Unary);
        assert_eq!(
            unary.operator_family(),
            SimpleTestOperatorFamily::StringUnary
        );
        assert!(
            unary
                .unary_operand_class()
                .is_some_and(|class| class.is_fixed_literal())
        );

        let binary = commands
            .iter()
            .find(|(text, _)| text == "[ left = right ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected binary test fact");
        assert_eq!(binary.shape(), SimpleTestShape::Binary);
        assert_eq!(
            binary.operator_family(),
            SimpleTestOperatorFamily::StringBinary
        );
        assert!(
            binary
                .binary_operand_classes()
                .is_some_and(|(left, right)| left.is_fixed_literal() && right.is_fixed_literal())
        );

        let literal_bang_binary = commands
            .iter()
            .find(|(text, _)| text == "[ ! = right ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected literal bang binary test fact");
        assert_eq!(literal_bang_binary.shape(), SimpleTestShape::Binary);
        assert!(!literal_bang_binary.is_effectively_negated());
        assert_eq!(
            literal_bang_binary.effective_shape(),
            SimpleTestShape::Binary
        );
        assert_eq!(
            literal_bang_binary.effective_operator_family(),
            SimpleTestOperatorFamily::StringBinary
        );
        assert!(
            literal_bang_binary
                .effective_operand_class(0)
                .zip(literal_bang_binary.effective_operand_class(2))
                .is_some_and(|(left, right)| {
                    left.is_fixed_literal() && right.is_fixed_literal()
                })
        );

        let negated_unary = commands
            .iter()
            .find(|(text, _)| text == "[ ! -n foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated unary test fact");
        assert_eq!(negated_unary.shape(), SimpleTestShape::Binary);
        assert!(negated_unary.is_effectively_negated());
        assert_eq!(negated_unary.effective_shape(), SimpleTestShape::Unary);
        assert_eq!(
            negated_unary.effective_operator_family(),
            SimpleTestOperatorFamily::StringUnary
        );
        assert!(
            negated_unary
                .effective_operand_class(1)
                .is_some_and(|class| class.is_fixed_literal())
        );

        let negated_binary = commands
            .iter()
            .find(|(text, _)| text == "[ ! left = right ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated binary test fact");
        assert_eq!(negated_binary.shape(), SimpleTestShape::Other);
        assert!(negated_binary.is_effectively_negated());
        assert_eq!(negated_binary.effective_shape(), SimpleTestShape::Binary);
        assert_eq!(
            negated_binary.effective_operator_family(),
            SimpleTestOperatorFamily::StringBinary
        );
        assert!(
            negated_binary
                .effective_operand_class(0)
                .zip(negated_binary.effective_operand_class(2))
                .is_some_and(|(left, right)| {
                    left.is_fixed_literal() && right.is_fixed_literal()
                })
        );

        let non_string_binary = commands
            .iter()
            .find(|(text, _)| text == "[ foo -eq 1 ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected numeric test fact");
        assert_eq!(non_string_binary.shape(), SimpleTestShape::Binary);
        assert_eq!(
            non_string_binary.operator_family(),
            SimpleTestOperatorFamily::Other
        );

        let missing_closer = commands
            .iter()
            .find(|(text, _)| text == "[ missing")
            .map(|(_, fact)| fact.simple_test());
        assert!(matches!(missing_closer, Some(None)));
    });
}

#[test]
fn simple_test_fact_tracks_escaped_negation_spans_for_fixes() {
    let source = "\
#!/bin/sh
[ \\! -f foo ]
test \\! -n foo
[ \\! foo = bar ]
[ \\! = right ]
[ ! -f foo ]
";

    with_facts(source, None, |_, facts| {
        let commands = facts
            .structural_commands()
            .map(|fact| (fact.span().slice(source).trim_end().to_owned(), fact))
            .collect::<Vec<_>>();

        let bracket_unary = commands
            .iter()
            .find(|(text, _)| text == "[ \\! -f foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .and_then(|simple_test| simple_test.escaped_negation_spans(source))
            .expect("expected escaped negation spans for bracket unary test");
        assert_eq!(
            (
                bracket_unary.0.slice(source).to_owned(),
                bracket_unary.1.slice(source).to_owned()
            ),
            ("-f".to_owned(), "\\".to_owned())
        );

        let builtin_unary = commands
            .iter()
            .find(|(text, _)| text == "test \\! -n foo")
            .and_then(|(_, fact)| fact.simple_test())
            .and_then(|simple_test| simple_test.escaped_negation_spans(source))
            .expect("expected escaped negation spans for test builtin");
        assert_eq!(
            (
                builtin_unary.0.slice(source).to_owned(),
                builtin_unary.1.slice(source).to_owned()
            ),
            ("\\!".to_owned(), "\\".to_owned())
        );

        let bracket_binary = commands
            .iter()
            .find(|(text, _)| text == "[ \\! foo = bar ]")
            .and_then(|(_, fact)| fact.simple_test())
            .and_then(|simple_test| simple_test.escaped_negation_spans(source))
            .expect("expected escaped negation spans for bracket binary test");
        assert_eq!(
            (
                bracket_binary.0.slice(source).to_owned(),
                bracket_binary.1.slice(source).to_owned()
            ),
            ("\\!".to_owned(), "\\".to_owned())
        );

        let non_operator = commands
            .iter()
            .find(|(text, _)| text == "[ \\! = right ]")
            .and_then(|(_, fact)| fact.simple_test())
            .and_then(|simple_test| simple_test.escaped_negation_spans(source));
        assert!(non_operator.is_none());
    });
}

#[test]
fn simple_test_fact_tracks_truthy_string_unary_and_string_binary_subexpressions() {
    let source = "\
#!/bin/sh
[ foo ]
[ ! bar ]
[ -z baz ]
[ ! -n qux ]
[ \"-n\" ]
[ \"-n\" foo ]
[ \"!\" \"-n\" qux ]
[ -a foo ]
[ -o foo ]
[ ! -a baz ]
[ ! -o quux ]
[ foo -o -z baz ]
[ -a foo -o -z baz ]
[ foo \"-o\" \"-z\" baz ]
[ -f file -a ! -z baz ]
[ lhs = rhs ]
[ lhs = rhs -a -z baz ]
";

    with_facts(source, None, |_, facts| {
        let commands = facts
            .structural_commands()
            .map(|fact| (fact.span().slice(source).trim_end().to_owned(), fact))
            .collect::<Vec<_>>();

        let truthy = commands
            .iter()
            .find(|(text, _)| text == "[ foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected truthy test fact");
        assert_eq!(
            truthy
                .truthy_expression_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["foo"]
        );

        let negated_truthy = commands
            .iter()
            .find(|(text, _)| text == "[ ! bar ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated truthy test fact");
        assert_eq!(
            negated_truthy
                .truthy_expression_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["bar"]
        );

        let unary = commands
            .iter()
            .find(|(text, _)| text == "[ -z baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected unary test fact");
        assert_eq!(
            unary
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("-z".to_owned(), "baz".to_owned())]
        );

        let negated_unary = commands
            .iter()
            .find(|(text, _)| text == "[ ! -n qux ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated unary test fact");
        assert_eq!(
            negated_unary
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("-n".to_owned(), "qux".to_owned())]
        );

        let quoted_literal = commands
            .iter()
            .find(|(text, _)| text == "[ \"-n\" ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected quoted literal test fact");
        assert_eq!(
            quoted_literal
                .truthy_expression_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["\"-n\""]
        );
        assert!(
            quoted_literal
                .string_unary_expression_words(source)
                .is_empty()
        );

        let negated_unary_a = commands
            .iter()
            .find(|(text, _)| text == "[ ! -a baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated unary -a test fact");
        assert!(negated_unary_a.truthy_expression_words(source).is_empty());
        assert!(
            negated_unary_a
                .string_unary_expression_words(source)
                .is_empty()
        );

        let negated_unary_o = commands
            .iter()
            .find(|(text, _)| text == "[ ! -o quux ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated unary -o test fact");
        assert!(negated_unary_o.truthy_expression_words(source).is_empty());
        assert!(
            negated_unary_o
                .string_unary_expression_words(source)
                .is_empty()
        );

        let quoted_unary = commands
            .iter()
            .find(|(text, _)| text == "[ \"-n\" foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected quoted unary test fact");
        assert_eq!(
            quoted_unary
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("\"-n\"".to_owned(), "foo".to_owned())]
        );

        let quoted_negated_unary = commands
            .iter()
            .find(|(text, _)| text == "[ \"!\" \"-n\" qux ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected quoted negated unary test fact");
        assert_eq!(
            quoted_negated_unary
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("\"-n\"".to_owned(), "qux".to_owned())]
        );

        let unary_and = commands
            .iter()
            .find(|(text, _)| text == "[ -a foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected unary -a test fact");
        assert!(unary_and.truthy_expression_words(source).is_empty());
        assert!(unary_and.string_unary_expression_words(source).is_empty());

        let unary_or = commands
            .iter()
            .find(|(text, _)| text == "[ -o foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected unary -o test fact");
        assert!(unary_or.truthy_expression_words(source).is_empty());
        assert!(unary_or.string_unary_expression_words(source).is_empty());

        let mixed = commands
            .iter()
            .find(|(text, _)| text == "[ foo -o -z baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected mixed test fact");
        assert_eq!(
            mixed
                .truthy_expression_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["foo"]
        );
        assert_eq!(
            mixed
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("-z".to_owned(), "baz".to_owned())]
        );

        let unary_and_then_connector = commands
            .iter()
            .find(|(text, _)| text == "[ -a foo -o -z baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected unary -a with connector test fact");
        assert!(
            unary_and_then_connector
                .truthy_expression_words(source)
                .is_empty()
        );
        assert_eq!(
            unary_and_then_connector
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("-z".to_owned(), "baz".to_owned())]
        );

        let quoted_connector = commands
            .iter()
            .find(|(text, _)| text == "[ foo \"-o\" \"-z\" baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected quoted connector test fact");
        assert_eq!(
            quoted_connector
                .truthy_expression_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["foo"]
        );
        assert_eq!(
            quoted_connector
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("\"-z\"".to_owned(), "baz".to_owned())]
        );

        let chained = commands
            .iter()
            .find(|(text, _)| text == "[ -f file -a ! -z baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected chained test fact");
        assert_eq!(
            chained
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("-z".to_owned(), "baz".to_owned())]
        );

        let binary = commands
            .iter()
            .find(|(text, _)| text == "[ lhs = rhs ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected binary test fact");
        assert!(binary.truthy_expression_words(source).is_empty());
        assert!(binary.string_unary_expression_words(source).is_empty());
        assert_eq!(
            binary
                .string_binary_expression_words(source)
                .into_iter()
                .map(|(left, operator, right)| {
                    (
                        left.span.slice(source).to_owned(),
                        operator.span.slice(source).to_owned(),
                        right.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("lhs".to_owned(), "=".to_owned(), "rhs".to_owned())]
        );

        let binary_then_unary = commands
            .iter()
            .find(|(text, _)| text == "[ lhs = rhs -a -z baz ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected binary plus unary test fact");
        assert_eq!(
            binary_then_unary
                .string_binary_expression_words(source)
                .into_iter()
                .map(|(left, operator, right)| {
                    (
                        left.span.slice(source).to_owned(),
                        operator.span.slice(source).to_owned(),
                        right.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("lhs".to_owned(), "=".to_owned(), "rhs".to_owned())]
        );
        assert_eq!(
            binary_then_unary
                .string_unary_expression_words(source)
                .into_iter()
                .map(|(operator, operand)| {
                    (
                        operator.span.slice(source).to_owned(),
                        operand.span.slice(source).to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![("-z".to_owned(), "baz".to_owned())]
        );
    });
}

#[test]
fn simple_test_fact_tracks_operator_expression_operands() {
    let source = "\
#!/bin/sh
[ foo ]
[ -d dir ]
[ lhs -eq rhs ]
[ -d one -o two = three ]
[ ! -e four -a five -nt six ]
";

    with_facts(source, None, |_, facts| {
        let commands = facts
            .structural_commands()
            .map(|fact| (fact.span().slice(source).trim_end().to_owned(), fact))
            .collect::<Vec<_>>();

        let truthy = commands
            .iter()
            .find(|(text, _)| text == "[ foo ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected truthy test fact");
        assert!(truthy.operator_expression_operand_words(source).is_empty());

        let unary = commands
            .iter()
            .find(|(text, _)| text == "[ -d dir ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected unary test fact");
        assert_eq!(
            unary
                .operator_expression_operand_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["dir"]
        );

        let binary = commands
            .iter()
            .find(|(text, _)| text == "[ lhs -eq rhs ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected binary test fact");
        assert_eq!(
            binary
                .operator_expression_operand_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["lhs", "rhs"]
        );

        let compound = commands
            .iter()
            .find(|(text, _)| text == "[ -d one -o two = three ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected compound test fact");
        assert_eq!(
            compound
                .operator_expression_operand_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["one", "two", "three"]
        );

        let negated_compound = commands
            .iter()
            .find(|(text, _)| text == "[ ! -e four -a five -nt six ]")
            .and_then(|(_, fact)| fact.simple_test())
            .expect("expected negated compound test fact");
        assert_eq!(
            negated_compound
                .operator_expression_operand_words(source)
                .into_iter()
                .map(|word| word.span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["four", "five", "six"]
        );
    });
}

#[test]
fn collects_compound_operator_spans_for_grouped_bracket_tests() {
    let source = "\
#!/bin/sh
[ ! '(' -f \"$left\" -o -f \"$right\" ')' ]
[ '(' '!' -f \"$quoted_left\" -o -f \"$quoted_right\" ')' ]
[ \"$a\" = 1 -a \\( \"$b\" = 2 -o \"$c\" = 3 \\) ]
";

    with_facts(source, None, |_, facts| {
        let tests = facts
            .structural_commands()
            .filter_map(|fact| fact.simple_test())
            .collect::<Vec<_>>();

        assert_eq!(tests.len(), 3);
        assert_eq!(
            tests[0]
                .compound_operator_spans(source)
                .into_iter()
                .map(|span| span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["-o".to_owned()]
        );
        assert_eq!(
            tests[1]
                .compound_operator_spans(source)
                .into_iter()
                .map(|span| span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["-o".to_owned()]
        );
        assert_eq!(
            tests[2]
                .compound_operator_spans(source)
                .into_iter()
                .map(|span| span.slice(source).to_owned())
                .collect::<Vec<_>>(),
            vec!["-a".to_owned(), "-o".to_owned()]
        );
    });
}

#[test]
fn skips_compound_operator_spans_for_malformed_grouped_bracket_tests() {
    let source = "\
#!/bin/sh
[ -n \"${TMPDIR-}\" -a '(' '(' -d \"${TMPDIR-}\" -a -w \"${TMPDIR-}\" ')' -o '!' '(' -d /tmp -a -w /tmp ')' ')' ]
";

    with_facts(source, None, |_, facts| {
        let test = facts
            .structural_commands()
            .find_map(|fact| fact.simple_test())
            .expect("expected malformed simple test fact");

        assert!(test.compound_operator_spans(source).is_empty());
    });
}

#[test]
fn skips_compound_operator_spans_for_quoted_negation_before_grouped_subexpressions() {
    let source = "\
#!/bin/sh
[ '(' '!' '(' -f \"$left\" -o -f \"$right\" ')' ')' ]
";

    with_facts(source, None, |_, facts| {
        let test = facts
            .structural_commands()
            .find_map(|fact| fact.simple_test())
            .expect("expected grouped simple test fact");

        assert!(test.compound_operator_spans(source).is_empty());
    });
}

#[test]
fn records_glued_closing_bracket_operand_spans_for_unary_tests() {
    let source = "\
#!/bin/sh
[ -d /tmp]
[ ! -a /tmp]
[ \"$dir\" = /tmp]
[ -d /tmp ]
";

    with_facts(source, None, |_, facts| {
        let commands = facts.structural_commands().collect::<Vec<_>>();
        assert_eq!(commands.len(), 4);

        assert_eq!(
            commands[0]
                .glued_closing_bracket_operand_span()
                .map(|span| (span.start.line, span.start.column)),
            Some((2, 6))
        );
        assert_eq!(
            commands[0]
                .glued_closing_bracket_insert_offset()
                .map(|offset| &source[offset..offset + 1]),
            Some("]")
        );
        assert_eq!(
            commands[1]
                .glued_closing_bracket_operand_span()
                .map(|span| (span.start.line, span.start.column)),
            Some((3, 8))
        );
        assert_eq!(
            commands[1]
                .glued_closing_bracket_insert_offset()
                .map(|offset| &source[offset..offset + 1]),
            Some("]")
        );
        assert_eq!(commands[2].glued_closing_bracket_operand_span(), None);
        assert_eq!(commands[2].glued_closing_bracket_insert_offset(), None);
        assert_eq!(commands[3].glued_closing_bracket_operand_span(), None);
        assert_eq!(commands[3].glued_closing_bracket_insert_offset(), None);
    });
}

#[test]
fn records_linebreak_in_test_fix_sites() {
    let source = "\
#!/bin/sh
if [ \"$x\" = y
]; then :; fi
if [ \"$x\" = y \\
]; then :; fi
if [ \"$x\" = y ]; then :; fi
";

    with_facts(source, None, |_, facts| {
        let commands = facts.structural_commands().collect::<Vec<_>>();
        let broken = commands
            .iter()
            .find(|fact| fact.static_utility_name_is("[") && fact.span().start.line == 2)
            .expect("expected broken bracket test");
        let continued = commands
            .iter()
            .find(|fact| fact.static_utility_name_is("[") && fact.span().start.line == 4)
            .expect("expected continued bracket test");
        let single_line = commands
            .iter()
            .find(|fact| fact.static_utility_name_is("[") && fact.span().start.line == 6)
            .expect("expected single-line bracket test");

        assert_eq!(
            broken
                .linebreak_in_test_anchor_span()
                .map(|span| (span.start.line, span.start.column)),
            Some((2, 14))
        );
        assert_eq!(
            broken
                .linebreak_in_test_insert_offset()
                .map(|offset| &source[offset..offset + 1]),
            Some("\n")
        );
        assert_eq!(continued.linebreak_in_test_anchor_span(), None);
        assert_eq!(continued.linebreak_in_test_insert_offset(), None);
        assert_eq!(single_line.linebreak_in_test_anchor_span(), None);
        assert_eq!(single_line.linebreak_in_test_insert_offset(), None);
    });
}

#[test]
fn collects_bare_command_name_assignment_spans() {
    let source = "\
#!/bin/sh
tool=grep
paths[$path]=set
tool=sh printf '%s\\n' hi
pager=cat \"$1\" -u perl
tool=\"grep\"
tool=git
tool=grep other=set printf '%s\\n' hi
f() {
  state=sh return 0
}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .bare_command_name_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "tool",
                "paths[$path]",
                "tool=sh printf '%s\\n' hi",
                "pager=cat \"$1\" -u perl",
                "state=sh return 0",
            ]
        );
    });
}

#[test]
fn backtick_command_name_spans_skip_argument_forms_but_keep_redirect_only_cases() {
    let source = "\
#!/bin/sh
`echo bare`
`echo bare` arg
`echo bare` 2>/dev/null
`echo bare` arg 2>/dev/null
true && `echo and_only`
true && `echo and_arg` arg
true && `echo and_redirect` 2>/dev/null
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .backtick_command_name_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "`echo bare`",
                "`echo bare`",
                "`echo and_only`",
                "`echo and_redirect`",
            ]
        );
    });
}

#[test]
fn marks_shellspec_parameter_region_empty_tests_as_suppressed() {
    let source = "\
Describe 'clone'
Parameters
  test
End

test
";

    with_facts(
        source,
        Some(Path::new(
            "/tmp/ko1nksm__shellspec__spec__core__clone_spec.sh",
        )),
        |_, facts| {
            let mut tests = facts
                .structural_commands()
                .filter_map(|fact| fact.simple_test().map(|simple| (fact.span(), simple)))
                .collect::<Vec<_>>();
            tests.sort_by_key(|(span, _)| span.start.line);

            assert_eq!(tests.len(), 2);
            assert!(tests[0].1.empty_test_suppressed());
            assert!(!tests[1].1.empty_test_suppressed());
        },
    );
}

#[test]
fn builds_loop_header_pipeline_and_list_facts() {
    let source = "\
#!/bin/bash
for file in $(printf '%s\\n' one two) \"$(command find . -type f)\" literal; do :; done
select choice in $(printf '%s\\n' a b) \"$(find . -type f)\" literal; do :; done
printf '%s\\n' 123 |& command kill -9 | tee out.txt
summary=$(printf '%s\\n' 456 | kill -TERM)
echo \"$(for nested in $(printf nested); do :; done)\"
true && false || printf '%s\\n' fallback
";

    with_facts(source, None, |_, facts| {
        assert_eq!(facts.for_headers().len(), 2);

        let top_level_for = &facts.for_headers()[0];
        assert!(!top_level_for.is_nested_word_command());
        assert_eq!(top_level_for.words().len(), 3);
        assert!(top_level_for.words()[0].has_unquoted_command_substitution());
        assert!(top_level_for.words()[1].contains_find_substitution());
        assert!(top_level_for.has_command_substitution());
        assert!(top_level_for.has_find_substitution());

        let nested_for = &facts.for_headers()[1];
        assert!(nested_for.is_nested_word_command());
        assert!(nested_for.words()[0].has_unquoted_command_substitution());

        let select = &facts.select_headers()[0];
        assert_eq!(select.words().len(), 3);
        assert!(select.words()[0].has_command_substitution());
        assert!(select.words()[1].contains_find_substitution());

        let pipeline_segments = facts
            .pipelines()
            .iter()
            .map(|pipeline| {
                pipeline
                    .segments()
                    .iter()
                    .map(|segment| {
                        segment
                            .effective_or_literal_name()
                            .expect("expected normalized pipeline segment name")
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            pipeline_segments,
            vec![
                vec!["printf".to_owned(), "kill".to_owned(), "tee".to_owned()],
                vec!["printf".to_owned(), "kill".to_owned()],
            ]
        );

        let first_pipeline = &facts.pipelines()[0];
        assert_eq!(
            first_pipeline
                .operators()
                .iter()
                .map(|operator| operator.op())
                .collect::<Vec<_>>(),
            vec![BinaryOp::PipeAll, BinaryOp::Pipe]
        );
        let first_segment = &first_pipeline.segments()[0];
        assert_eq!(
            facts
                .command(first_segment.command_id())
                .effective_or_literal_name(),
            Some("printf")
        );

        let list = facts.lists().first().expect("expected list fact");
        assert_eq!(
            list.operators()
                .iter()
                .map(|operator| operator.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["&&", "||"]
        );
        assert_eq!(
            list.mixed_short_circuit_span()
                .map(|span| span.slice(source)),
            Some("&&")
        );
        assert_eq!(
            list.mixed_short_circuit_kind(),
            Some(crate::facts::MixedShortCircuitKind::Fallthrough)
        );
        assert_eq!(
            list.segments()
                .iter()
                .map(|segment| segment.kind())
                .collect::<Vec<_>>(),
            vec![
                crate::facts::ListSegmentKind::Condition,
                crate::facts::ListSegmentKind::Condition,
                crate::facts::ListSegmentKind::Other,
            ]
        );
    });
}

#[test]
fn classifies_mixed_short_circuit_lists_by_shape() {
    let source = "\
#!/bin/sh
[ \"$x\" = foo ] && [ \"$x\" = bar ] || [ \"$x\" = baz ]
[ -n \"$x\" ] && out=foo || out=bar
[ -n \"$x\" ] || out=foo && out=bar
[ \"$dir\" = vendor ] && mv go-* \"$dir\" || mv pkg-* \"$dir\"
";

    with_facts(source, None, |_, facts| {
        assert_eq!(facts.lists().len(), 4);
        assert_eq!(
            facts
                .lists()
                .iter()
                .map(|list| list.mixed_short_circuit_kind())
                .collect::<Vec<_>>(),
            vec![
                Some(crate::facts::MixedShortCircuitKind::TestChain),
                Some(crate::facts::MixedShortCircuitKind::AssignmentTernary),
                Some(crate::facts::MixedShortCircuitKind::Fallthrough),
                Some(crate::facts::MixedShortCircuitKind::Fallthrough),
            ]
        );
    });
}

#[test]
fn preserves_fallback_command_names_inside_command_substitutions() {
    let source = "\
#!/bin/sh
echo \"\\\"$BUILDSCRIPT\\\" --library $(test \"${PKG_DIR%/*}\" = \"gpkg\" && echo \"glibc\" || echo \"bionic\")\"
";

    with_facts(source, None, |_, facts| {
        let list = facts.lists().first().expect("expected mixed list");
        let names = list
            .segments()
            .iter()
            .map(|segment| {
                facts
                    .command(segment.command_id())
                    .effective_or_literal_name()
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                Some("test".to_owned()),
                Some("echo".to_owned()),
                Some("echo".to_owned()),
            ]
        );
    });
}

#[test]
fn flagged_declaration_assignments_still_classify_as_assignment_segments() {
    let source = "\
#!/bin/bash
[ -n \"$x\" ] && declare -r out=foo || declare -r out=bar
true && declare -x flag=1
";

    with_facts(source, None, |_, facts| {
        assert_eq!(facts.lists().len(), 2);

        let ternary = &facts.lists()[0];
        assert_eq!(
            ternary.mixed_short_circuit_kind(),
            Some(crate::facts::MixedShortCircuitKind::AssignmentTernary)
        );
        assert_eq!(
            ternary
                .segments()
                .iter()
                .map(|segment| segment.assignment_target())
                .collect::<Vec<_>>(),
            vec![None, Some("out"), Some("out")]
        );
        assert_eq!(
            ternary
                .segments()
                .iter()
                .map(|segment| segment.assignment_is_declaration())
                .collect::<Vec<_>>(),
            vec![false, true, true]
        );

        let shortcut = &facts.lists()[1];
        assert_eq!(
            shortcut
                .segments()
                .iter()
                .map(|segment| segment.kind())
                .collect::<Vec<_>>(),
            vec![
                crate::facts::ListSegmentKind::Condition,
                crate::facts::ListSegmentKind::AssignmentOnly,
            ]
        );
        assert_eq!(shortcut.segments()[1].assignment_target(), Some("flag"));
        assert!(shortcut.segments()[1].assignment_is_declaration());
    });
}

#[test]
fn builds_loop_header_ls_substitution_detection() {
    let source = "\
#!/bin/bash
for entry in $(ls); do :; done
for entry in $(command ls); do :; done
for entry in $(find . -type f); do :; done
";

    with_facts(source, None, |_, facts| {
        let words = facts.for_headers()[0].words();
        assert!(words[0].has_unquoted_command_substitution());
        assert!(words[0].contains_ls_substitution());

        let command_ls = facts.for_headers()[1].words();
        assert!(command_ls[0].has_unquoted_command_substitution());
        assert!(!command_ls[0].contains_ls_substitution());

        let find_words = facts.for_headers()[2].words();
        assert!(find_words[0].has_unquoted_command_substitution());
        assert!(!find_words[0].contains_ls_substitution());
    });
}

#[test]
fn builds_loop_header_find_substitution_detection_for_find_exec_forms() {
    let source = "\
#!/bin/bash
for entry in $(find . -type f -exec grep -Pl '\\r$' {} \\;); do :; done
for entry in $(command find . -type f -exec basename {} \\;); do :; done
";

    with_facts(source, None, |_, facts| {
        let first = facts.for_headers()[0].words();
        assert!(first[0].has_unquoted_command_substitution());
        assert!(first[0].contains_find_substitution());

        let second = facts.for_headers()[1].words();
        assert!(second[0].has_unquoted_command_substitution());
        assert!(second[0].contains_find_substitution());
    });
}

#[test]
fn builds_loop_header_line_oriented_substitution_detection() {
    let source = "\
#!/bin/bash
for entry in $(cat input.txt); do :; done
for entry in $(grep foo input.txt | cut -d: -f1); do :; done
for entry in $(printf '%s\\n' a b); do :; done
for entry in $(echo a b | rev); do :; done
for entry in $(cat input.txt | head -n1); do :; done
for entry in $(find . -type f); do :; done
";

    with_facts(source, None, |_, facts| {
        let words = facts
            .for_headers()
            .iter()
            .map(|header| header.words()[0].contains_line_oriented_substitution())
            .collect::<Vec<_>>();

        assert_eq!(words, vec![true, true, false, false, false, false]);
    });
}

#[test]
fn keeps_mixed_word_lists_and_find_exec_substitutions_out_of_line_oriented_detection() {
    let source = "\
#!/bin/bash
for entry in literal $(cat input.txt); do :; done
for entry in $(find . -type f -exec grep -Pl '\\r$' {} \\;); do :; done
";

    with_facts(source, None, |_, facts| {
        assert_eq!(facts.for_headers().len(), 2);
        assert!(facts.for_headers()[0].words()[1].contains_line_oriented_substitution());
        assert!(!facts.for_headers()[1].words()[0].contains_line_oriented_substitution());
    });
}

#[test]
fn zsh_for_headers_only_track_iteration_words() {
    let source = "\
#!/usr/bin/env zsh
for key value in $(printf '%s\\n' a b) literal; do :; done
for version ($versions); do :; done
";

    with_facts_dialect(
        source,
        Some(Path::new("script.zsh")),
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            assert_eq!(facts.for_headers().len(), 2);

            let first = &facts.for_headers()[0];
            assert_eq!(first.words().len(), 2);
            assert!(first.words()[0].has_command_substitution());
            assert_eq!(
                first
                    .words()
                    .iter()
                    .map(|word| word.word().span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$(printf '%s\\n' a b)", "literal"]
            );

            let second = &facts.for_headers()[1];
            assert_eq!(second.words().len(), 1);
            assert_eq!(second.words()[0].word().span.slice(source), "$versions");
        },
    );
}

#[test]
fn builds_conditional_facts_with_root_normalization_and_nested_inventory() {
    let source = "\
#!/bin/bash
[[ ( ( -z foo ) ) ]]
[[ foo && -n \"$bar\" && left == right && $value =~ ^\"foo\"bar$ && left == *.sh && left == $rhs ]]
";

    with_facts(source, None, |_, facts| {
        let conditionals = facts
            .structural_commands()
            .filter_map(|fact| fact.conditional())
            .collect::<Vec<_>>();

        let root_unary = conditionals[0];
        match root_unary.root() {
            ConditionalNodeFact::Unary(unary) => {
                assert_eq!(
                    unary.operator_family(),
                    ConditionalOperatorFamily::StringUnary
                );
                assert!(unary.operand().class().is_fixed_literal());
            }
            other => panic!("expected unary root, got {other:?}"),
        }

        let logical = conditionals[1];
        match logical.root() {
            ConditionalNodeFact::Binary(binary) => {
                assert_eq!(binary.operator_family(), ConditionalOperatorFamily::Logical);
            }
            other => panic!("expected logical root, got {other:?}"),
        }

        assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::BareWord(word) if word.operand().class().is_fixed_literal())));
        assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::Unary(unary) if unary.operator_family() == ConditionalOperatorFamily::StringUnary)));
        assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::Binary(binary) if binary.operator_family() == ConditionalOperatorFamily::StringBinary && binary.right().class().is_fixed_literal())));
        assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::Binary(binary) if binary.operator_family() == ConditionalOperatorFamily::StringBinary && !binary.right().class().is_fixed_literal())));
        assert!(logical.nodes().iter().any(|node| matches!(
            node,
            ConditionalNodeFact::Binary(binary)
                if matches!(binary.op(), ConditionalBinaryOp::PatternEq)
                    && binary
                        .right()
                        .word()
                        .is_some_and(|word| word.span.slice(source) == "$rhs")
        )));

        let regex = logical.regex_nodes().next().expect("expected regex node");
        assert_eq!(regex.operator_family(), ConditionalOperatorFamily::Regex);
        assert_eq!(
            regex.right().word().map(|word| word.span.slice(source)),
            Some("^\"foo\"bar$")
        );
        assert!(logical.mixed_logical_operators().is_empty());
        assert!(
            regex
                .right()
                .quote()
                .is_some_and(|quote| quote != crate::rules::common::word::WordQuote::Unquoted)
        );
    });
}

#[test]
fn tab_stripped_heredoc_substitutions_after_earlier_heredocs_keep_command_spans_intact() {
    let source = "\
#!/bin/bash
case \"${tag_type}\" in
  newest-tag)
\t:
\t;;
  latest-release-tag)
\t:
\t;;
  latest-regex)
\t:
\t;;
  *)
\ttermux_error_exit <<-EndOfError
\t\tERROR: Invalid TERMUX_PKG_UPDATE_TAG_TYPE: '${tag_type}'.
\t\tAllowed values: 'newest-tag', 'latest-release-tag', 'latest-regex'.
\tEndOfError
\t;;
esac

case \"${http_code}\" in
  404)
\ttermux_error_exit <<-EndOfError
\t\tNo '${tag_type}' found. (${api_url})
\t\tHTTP code: ${http_code}
\t\tTry using '$(
\t\t\tif [[ \"${tag_type}\" == \"newest-tag\" ]]; then
\t\t\t\techo \"latest-release-tag\"
\t\t\telse
\t\t\t\techo \"newest-tag\"
\t\t\tfi
\t\t)'.
\tEndOfError
\t;;
esac
";

    with_facts(source, None, |_, facts| {
        let conditional = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "[[ \"${tag_type}\" == \"newest-tag\" ]]")
            .expect("expected nested heredoc conditional command");

        let conditional_fact = conditional
            .conditional()
            .expect("expected conditional fact for nested heredoc command");

        match conditional_fact.root() {
            ConditionalNodeFact::Binary(binary) => {
                assert_eq!(
                    binary.operator_family(),
                    ConditionalOperatorFamily::StringBinary
                );
                assert_eq!(
                    binary.left().word().map(|word| word.span.slice(source)),
                    Some("\"${tag_type}\"")
                );
                assert_eq!(
                    binary.right().word().map(|word| word.span.slice(source)),
                    Some("\"newest-tag\"")
                );
            }
            other => panic!("expected binary root, got {other:?}"),
        }

        let latest_release = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "echo \"latest-release-tag\"\n")
            .expect("expected latest-release echo command");
        assert!(latest_release.simple_test().is_none());

        let newest_tag = facts
            .commands()
            .iter()
            .find(|fact| fact.span().slice(source) == "echo \"newest-tag\"\n")
            .expect("expected newest-tag echo command");
        assert!(newest_tag.simple_test().is_none());
    });
}

#[test]
fn keeps_parenthesized_logical_groups_separate_for_mixed_operator_detection() {
    let source = "\
#!/bin/bash
[[ -n $a && -n $b || -n $c ]]
[[ -n $a && ( -n $b || -n $c ) ]]
[[ ( -n $a && -n $b || -n $c ) && -n $d ]]
[[ -n $a && -n $b || -n $c && -n $d ]]
";

    with_facts(source, None, |_, facts| {
        let conditionals = facts
            .structural_commands()
            .filter_map(|fact| fact.conditional())
            .collect::<Vec<_>>();

        let first = &conditionals[0].mixed_logical_operators()[0];
        assert_eq!(first.operator_span().slice(source), "||");
        assert_eq!(
            first
                .grouped_subexpression_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-n $a && -n $b"]
        );
        assert!(conditionals[1].mixed_logical_operators().is_empty());
        let third = &conditionals[2].mixed_logical_operators()[0];
        assert_eq!(third.operator_span().slice(source), "||");
        assert_eq!(
            third
                .grouped_subexpression_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-n $a && -n $b"]
        );
        let fourth = &conditionals[3].mixed_logical_operators()[0];
        assert_eq!(fourth.operator_span().slice(source), "||");
        assert_eq!(
            fourth
                .grouped_subexpression_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-n $a && -n $b", "-n $c && -n $d"]
        );
    });
}

#[test]
fn builds_conditional_portability_fact_buckets_from_shared_command_scans() {
    let source = "\
#!/bin/bash
if test left == right; then
  :
elif [[ $x == y ]]; then
  :
fi
[ left == right ]
[[ $OSTYPE == *@(linux|freebsd)* ]]
[ \"$x\" = @(foo|bar) ]
[[ $words[2] = */ ]]
[ $tools[kops] ]
[[ $x < z ]]
[[ $x > y ]]
[[ $x =~ y ]]
[[ -v assoc[$key] ]]
[[ -a file ]]
[[ -o noclobber ]]
[ -k \"$file\" ]
test -O \"$file\"
";

    with_facts(source, None, |_, facts| {
        let portability = facts.conditional_portability();

        assert_eq!(portability.double_bracket_in_sh().len(), 9);
        assert_eq!(
            portability
                .if_elif_bash_test()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[[ $x == y ]]"]
        );
        assert_eq!(
            portability
                .test_equality_operator()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["test left == right", "==", "==", "=="]
        );
        assert_eq!(
            portability
                .extglob_in_test()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["@(linux|freebsd)", "@(foo|bar)"]
        );
        assert_eq!(
            portability
                .array_subscript_test()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$tools[kops]"]
        );
        assert_eq!(
            portability
                .array_subscript_condition()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$words[2]", "assoc[$key]"]
        );
        assert_eq!(
            portability
                .lexical_comparison_in_double_bracket()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["<", ">"]
        );
        assert_eq!(
            portability
                .regex_match_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["=~"]
        );
        assert_eq!(
            portability
                .v_test_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-v"]
        );
        assert_eq!(
            portability
                .a_test_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-a"]
        );
        assert_eq!(
            portability
                .option_test_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-o"]
        );
        assert_eq!(
            portability
                .sticky_bit_test_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-k"]
        );
        assert_eq!(
            portability
                .ownership_test_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["test -O \"$file\""]
        );
    });
}

#[test]
fn builds_conditional_portability_pattern_buckets_from_surface_and_word_sources() {
    let source = "\
#!/bin/bash
echo @(foo|bar)
case \"$x\" in @(zip|tar)) : ;; esac
trimmed=${name%@($suffix|zz)}
echo [^a]*
trimmed=${value#[^b]*}
pkgopts=${value//[^d]/_}
for item in [^c]*; do :; done
";

    with_facts(source, None, |_, facts| {
        let extglobs = facts
            .conditional_portability()
            .extglob_in_sh()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert!(extglobs.contains(&"@(foo|bar)"));
        assert!(extglobs.contains(&"@(zip|tar)"));
        assert!(extglobs.contains(&"@($suffix|zz)"));

        let caret_negations = facts
            .conditional_portability()
            .caret_negation_in_bracket()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert!(caret_negations.contains(&"[^a]"));
        assert!(caret_negations.contains(&"[^c]"));
        assert!(!caret_negations.contains(&"[^b]"));
        assert!(!caret_negations.contains(&"[^d]"));
    });
}
