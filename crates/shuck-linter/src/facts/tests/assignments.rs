use super::*;
use crate::AmbientShellOptions;

#[test]
fn indexes_scalar_bindings_from_assignments_and_declarations() {
    let source = "#!/bin/bash\nfoo=1\nprintf '%s\\n' \"$foo\"\nexport bar=2\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    match &output.file.body[0].command {
        shuck_ast::Command::Simple(_) => {}
        _ => panic!("expected simple command"),
    };
    let first_binding_id = semantic.bindings_for(&Name::from("foo"))[0];
    assert_eq!(
        facts
            .binding_value(first_binding_id)
            .and_then(|value| value.scalar_word())
            .map(|word| word.span.slice(source)),
        Some("1")
    );

    match &output.file.body[2].command {
        shuck_ast::Command::Decl(command) => match &command.operands[0] {
            shuck_ast::DeclOperand::Assignment(_) => {}
            _ => panic!("expected declaration assignment"),
        },
        _ => panic!("expected declaration command"),
    };
    let second_binding_id = semantic.bindings_for(&Name::from("bar"))[0];
    assert_eq!(
        facts
            .binding_value(second_binding_id)
            .and_then(|value| value.scalar_word())
            .map(|word| word.span.slice(source)),
        Some("2")
    );
}

#[test]
fn indexes_loop_bindings_from_for_words() {
    let source = "#!/bin/bash\nfor i in 16 32 64; do printf '%s\\n' \"$i\"; done\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    let loop_binding_span = match &output.file.body[0].command {
        shuck_ast::Command::Compound(shuck_ast::CompoundCommand::For(command)) => {
            command.targets[0].span
        }
        _ => panic!("expected for command"),
    };
    let loop_binding_id = semantic
        .visible_binding(&Name::from("i"), loop_binding_span)
        .expect("expected i loop binding")
        .id;

    assert_eq!(
        facts
            .binding_value(loop_binding_id)
            .and_then(|value| value.loop_words())
            .expect("expected loop binding values")
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["16", "32", "64"]
    );
}

#[test]
fn marks_conditional_assignment_shortcuts_on_binding_values() {
    let source = "\
#!/bin/bash
check() { return 0; }
true && w='-w' || w=''
check && opt='-o' || opt=''
if true; then flag='-f'; else flag=''; fi
check && one_sided='-x'
one_sided='-b' || one_sided='-y'
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    let shortcut_bindings = semantic
        .bindings_for(&Name::from("w"))
        .iter()
        .copied()
        .map(|binding_id| {
            facts
                .binding_value(binding_id)
                .expect("expected w binding value fact")
                .conditional_assignment_shortcut()
        })
        .collect::<Vec<_>>();
    assert_eq!(shortcut_bindings, vec![true, true]);

    let command_shortcut_bindings = semantic
        .bindings_for(&Name::from("opt"))
        .iter()
        .copied()
        .map(|binding_id| {
            facts
                .binding_value(binding_id)
                .expect("expected opt binding value fact")
                .conditional_assignment_shortcut()
        })
        .collect::<Vec<_>>();
    assert_eq!(command_shortcut_bindings, vec![true, true]);

    let flag_bindings = semantic
        .bindings_for(&Name::from("flag"))
        .iter()
        .copied()
        .map(|binding_id| {
            facts
                .binding_value(binding_id)
                .expect("expected flag binding value fact")
                .conditional_assignment_shortcut()
        })
        .collect::<Vec<_>>();
    assert_eq!(flag_bindings, vec![false, false]);

    let one_sided_bindings = semantic
        .bindings_for(&Name::from("one_sided"))
        .iter()
        .copied()
        .map(|binding_id| {
            facts
                .binding_value(binding_id)
                .expect("expected one_sided binding value fact")
                .one_sided_short_circuit_assignment()
        })
        .collect::<Vec<_>>();
    assert_eq!(one_sided_bindings, vec![true, false, false]);
}

#[test]
fn ignores_command_prefix_assignments_when_indexing_binding_values() {
    let source = "\
#!/bin/bash
foo=stable
foo=ephemeral tool
printf '%s\\n' \"$foo\"
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    let foo_bindings = semantic.bindings_for(&Name::from("foo"));
    assert_eq!(foo_bindings.len(), 1);
    assert_eq!(
        facts
            .binding_value(foo_bindings[0])
            .and_then(|value| value.scalar_word())
            .map(|word| word.span.slice(source)),
        Some("stable")
    );
}

#[test]
fn declaration_assignment_values_attach_to_the_declared_binding() {
    let source = "\
#!/bin/bash
f() {
  (
    value=shadow
    local value=chosen
    printf '%s\\n' \"$value\"
  )
}
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    let shadow_binding = semantic
        .bindings_for(&Name::from("value"))
        .iter()
        .copied()
        .find(|binding_id| semantic.binding(*binding_id).attributes.is_empty())
        .expect("expected subshell shadow binding");
    let local_binding = semantic
        .bindings_for(&Name::from("value"))
        .iter()
        .copied()
        .find(|binding_id| {
            semantic
                .binding(*binding_id)
                .attributes
                .contains(BindingAttributes::LOCAL)
        })
        .expect("expected local declaration binding");

    assert_eq!(
        facts
            .binding_value(shadow_binding)
            .and_then(|value| value.scalar_word())
            .map(|word| word.span.slice(source)),
        Some("shadow")
    );
    assert_eq!(
        facts
            .binding_value(local_binding)
            .and_then(|value| value.scalar_word())
            .map(|word| word.span.slice(source)),
        Some("chosen")
    );
}

#[test]
fn collects_plus_equals_assignment_spans() {
    let source = "\
#!/bin/sh
x+=64
arr+=(one two)
readonly r+=1
index[1+2]+=3
complex[$((i+=1))]+=x
(( i += 1 ))
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .plus_equals_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["x", "arr", "r", "index[1+2]", "complex[$((i+=1))]"]
        );
    });
}

#[test]
fn collects_assignment_like_command_name_spans() {
    let source = r#"#!/bin/bash
+YYYY="$( date +%Y )"
export +MONTH=12
network.wan.proto='dhcp'
@VAR@=$(. /etc/profile >/dev/null 2>&1; echo "${@VAR@}")
echo +YEAR=2024
+1=bad
name+=still_ok
"#;

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .assignment_like_command_name_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "+YYYY=\"$( date +%Y )\"",
                "+MONTH=12",
                "network.wan.proto='dhcp'",
                "@VAR@=$(. /etc/profile >/dev/null 2>&1; echo \"${@VAR@}\")",
            ]
        );
    });
}

#[test]
fn ignores_assignment_like_text_after_literal_arrow_prefix() {
    let source = r#"#!/bin/bash
rvm_info="
  bash: \"$(command -v bash) => $(version_for bash)\"
  zsh:  \"$(command -v zsh) => $(version_for zsh)\"
"
"#;

    with_facts(source, None, |_, facts| {
        assert!(
            facts.assignment_like_command_name_spans().is_empty(),
            "quoted arrow text should not look like an assignment command name"
        );
    });
}

#[test]
fn collects_broken_assoc_key_spans_from_compound_array_assignments() {
    let source = "#!/bin/bash\ndeclare -A table=([left]=1 [right=2)\nother=([ok]=1 [broken=2)\ndeclare -A third=([$(echo ])=3)\ndeclare -A valid=([$(printf key)]=4)\ndeclare -a nums=([0]=1 [1=2)\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert_eq!(
        facts
            .broken_assoc_key_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["[right=2", "[broken=2", "[$(echo ])=3"]
    );
}

#[test]
fn collects_comma_array_assignment_spans_from_compound_values() {
    let source = "#!/bin/bash\na=(alpha,beta)\nb=(\"alpha,beta\")\nc=({x,y})\nd=([k]=v, [q]=w)\ne=(x,$y)\nf=(x\\, y)\ng=({$XDG_CONFIG_HOME,$HOME}/{alacritty,}/{.,}alacritty.ym?)\nh=(foo,{x,y},bar)\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert_eq!(
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec![
            "(alpha,beta)",
            "([k]=v, [q]=w)",
            "(x,$y)",
            "(x\\, y)",
            "(foo,{x,y},bar)"
        ]
    );
}

#[test]
fn collects_ifs_literal_backslash_assignment_value_spans() {
    let source = "\
#!/bin/bash
IFS='\\n'
export IFS=\"x\\n\"
while IFS='\\ \\|\\ ' read -r serial board_serial; do
  :
done < /dev/null
declare IFS='prefix\\nsuffix'
IFS=$'\\n'
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .ifs_literal_backslash_assignment_value_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'\\n'", "\"x\\n\"", "'\\ \\|\\ '", "'prefix\\nsuffix'"]
        );
    });
}

#[test]
fn ignores_commas_after_even_backslashes_before_quote_regions() {
    let source = "#!/bin/bash\na=(x\\\\\",y\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_ansi_c_quoted_array_elements() {
    let source = "#!/bin/bash\na=($'a\\'b,c')\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_quoted_command_substitution_array_elements() {
    let source = "#!/bin/bash\nf() {\n\tlocal -a graphql_request=(\n\t\t-X POST\n\t\t-d \"$(\n\t\t\tcat <<-EOF | tr '\\n' ' '\n\t\t\t\t{\"query\":\"field, direction\"}\n\t\t\tEOF\n\t\t)\"\n\t)\n}\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_separator_started_command_substitution_comments() {
    let source = "#!/bin/bash\na=(\"$(printf '%s' x;# comment with ) and ,\nprintf '%s' y\n)\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_grouped_command_substitution_comments() {
    let source = "#!/bin/bash\na=(\"$( (# comment with )\nprintf %s 1,2\n) )\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_compact_grouped_command_substitution_comments() {
    let source = "#!/bin/bash\na=(\"$( (#comment with )\nprintf %s 1,2\n) )\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_command_substitution_case_patterns() {
    let source = "#!/bin/bash\na=(\"$(case $kind in\nalpha) printf %s 1,2 ;;\nesac)\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_piped_heredoc_command_substitution_array_elements() {
    let source =
        "#!/bin/bash\na=(\"$(cat <<EOF|tr '\\n' ' '\n{\"query\":\"field, direction\"}\nEOF\n)\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_parameter_expansions_with_right_parens_in_command_substitutions() {
    let source = "#!/bin/bash\na=($(printf %s ${x//foo/)},1))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_parameter_expansions_with_literal_braces() {
    let source = "#!/bin/bash\na=(${x/a,b/{})\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_parameter_expansions_with_ansi_c_single_quotes() {
    let source = "#!/bin/bash\na=(${x/$'a\\'b'/c,d})\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_case_pattern_comments_after_right_parens() {
    let source =
        "#!/bin/bash\na=($(case $kind in\na)# comment with esac )\nprintf %s 1,2 ;;\nesac\n))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_process_substitution_array_elements() {
    let source = "#!/bin/bash\na=(<(printf %s 1,2))\nb=(>(printf %s 3,4))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_comments_after_quoted_double_parens() {
    let source = "#!/bin/bash\na=($(printf '((' # comment with )\nprintf %s 1,2\n))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_arithmetic_shift_command_substitutions() {
    let source = "#!/bin/bash\na=($( ((x<<2))\nprintf %s 1,2\n))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_multiline_command_substitution_scanner_edge_cases() {
    let source = "\
#!/bin/bash
a=($(printf '((' # comment with )
printf %s 1,2
))
b=($( ((x<<2))
printf %s 3,4
))
c=($( (case $kind in
a) printf %s 5,6 ;;
esac
) ))
d=(\"$( (#comment with )
printf %s 7,8
) )\")
e=($(printf %s 9,10; echo case in))
f=($(printf %s $'a\\'b'; printf %s 11,12))
g=($(printf %s `echo foo)`; printf %s 13,14))
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_nested_case_patterns_in_command_substitutions() {
    let source = "#!/bin/bash\na=($( (case $kind in\na) printf %s 1,2 ;;\nesac\n) ))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_command_substitutions_with_plain_case_words() {
    let source = "#!/bin/bash\na=($(printf %s 1,2; echo case in))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_command_substitutions_with_ansi_c_single_quotes() {
    let source = "#!/bin/bash\na=($(printf %s $'a\\'b'; printf %s 1,2))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_command_substitutions_with_backticks() {
    let source = "#!/bin/bash\na=($(printf %s `echo foo)`; printf %s 1,2))\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_backticks_inside_parameter_expansions() {
    let source = "#!/bin/bash\na=(${x/`echo }`/a,b})\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_inside_process_substitutions_inside_parameter_expansions() {
    let source = "#!/bin/bash\na=(${x/<(echo })/foo,bar})\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_after_backticks_inside_parameter_expansions_in_command_substitutions() {
    let source = "#!/bin/bash\na=(\"$(printf %s ${x/`echo }`/foo)},1)\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ignores_commas_after_process_substitutions_inside_parameter_expansions_in_command_substitutions()
{
    let source = "#!/bin/bash\na=(\"$(printf %s ${x/<(echo })/foo)},1)\")\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
    );

    assert!(
        facts.comma_array_assignment_spans().is_empty(),
        "{:#?}",
        facts
            .comma_array_assignment_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn bash_pipefail_skips_top_level_pipeline_subshell_use_sites() {
    let source = "\
#!/usr/bin/env bash
set -o pipefail
count=0
printf '%s\\n' x | while read -r _; do count=1; done
echo \"$count\"
";

    assert!(subshell_later_use_slices(source, ShellDialect::Bash).is_empty());
}

#[test]
fn bash_pipefail_keeps_enclosing_command_substitution_use_sites() {
    let source = "\
#!/usr/bin/env bash
set -o pipefail
value=outer
snapshot=\"$(value=inner | cat)\"
echo \"$value\"
";

    assert_eq!(
        subshell_later_use_slices(source, ShellDialect::Bash),
        vec!["$value"]
    );
}

#[test]
fn subshell_loop_assignment_sites_use_loop_keyword_spans() {
    let source = "\
#!/usr/bin/env bash
(for value in one two; do :; done)
printf '%s\\n' \"$value\"
{ select choice in one two; do break; done; } | cat
printf '%s\\n' \"$choice\"
";

    assert_eq!(
        subshell_assignment_slices(source, ShellDialect::Bash),
        vec!["for", "select"]
    );
}

#[test]
fn uninitialized_declarations_do_not_hide_subshell_use_sites() {
    let source = "\
#!/usr/bin/env bash
demo() {
  (value=inner)
  local value
  printf '%s\\n' \"${value:-}\"
}
";

    assert_eq!(
        subshell_later_use_slices(source, ShellDialect::Bash),
        vec!["${value:-}"]
    );
}

fn subshell_assignment_slices(source: &str, shell: ShellDialect) -> Vec<&str> {
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, shell);
    let facts = LinterFacts::build_with_shell_and_ambient_shell_options(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
        shell,
        AmbientShellOptions::default(),
    );

    facts
        .subshell_assignment_sites()
        .iter()
        .map(|site| site.span.slice(source))
        .collect()
}

fn subshell_later_use_slices(source: &str, shell: ShellDialect) -> Vec<&str> {
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let file_context = classify_file_context(source, None, shell);
    let facts = LinterFacts::build_with_shell_and_ambient_shell_options(
        &output.file,
        &output.arena_file,
        source,
        &semantic,
        &indexer,
        &file_context,
        shell,
        AmbientShellOptions::default(),
    );

    facts
        .subshell_later_use_sites()
        .iter()
        .map(|site| site.span.slice(source))
        .collect()
}
