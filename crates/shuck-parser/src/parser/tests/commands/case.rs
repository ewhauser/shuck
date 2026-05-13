use super::*;

#[test]
fn test_parse_collects_zsh_case_group_facts_in_posix_mode() {
    let input = "case $x in\n  foo_(a|b)_*) echo ok ;;\nesac\n";
    let parsed = Parser::with_dialect(input, ShellDialect::Posix).parse();

    assert_eq!(parsed.syntax_facts.zsh_case_group_parts.len(), 1);
    assert_eq!(
        parsed.syntax_facts.zsh_case_group_parts[0].pattern_part_index,
        1
    );
    assert_eq!(
        parsed.syntax_facts.zsh_case_group_parts[0]
            .span
            .slice(input),
        "(a|b)"
    );
}

#[test]
fn test_case_patterns_consume_segmented_tokens_directly() {
    let input = "case $x in foo\"bar\"|'baz'qux) echo hi ;; esac";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let patterns = &command.cases[0].patterns;
    assert_eq!(patterns.len(), 2);

    assert_eq!(patterns[0].render(input), "foobar");
    assert_eq!(patterns[0].parts.len(), 2);
    assert_eq!(
        pattern_part_slices(&patterns[0], input),
        vec!["foo", "\"bar\""]
    );
    assert!(matches!(
        &patterns[0].parts[1].kind,
        PatternPart::Word(word) if is_fully_quoted(word)
    ));

    assert_eq!(patterns[1].render(input), "bazqux");
    assert_eq!(patterns[1].parts.len(), 2);
    assert_eq!(
        pattern_part_slices(&patterns[1], input),
        vec!["'baz'", "qux"]
    );
    assert!(matches!(
        &patterns[1].parts[0].kind,
        PatternPart::Word(word) if is_fully_quoted(word)
    ));
}

#[test]
fn test_case_accepts_literal_left_brace_pattern() {
    let input = concat!(
        "case \"$word\" in\n",
        "  \\(\\)) : ;;\n",
        "  {) : ;;\n",
        "  :) : ;;\n",
        "esac\n",
    );
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases.len(), 3);
    assert_eq!(command.cases[0].patterns[0].render_syntax(input), "\\(\\)");
    assert_eq!(command.cases[1].patterns[0].render_syntax(input), "{");
    assert_eq!(command.cases[2].patterns[0].render_syntax(input), ":");
}

#[test]
fn test_zsh_case_accepts_suffix_bare_group_pattern() {
    let input = concat!(
        "case \"$mode\" in\n",
        "  plugin::(disable|enable|load)) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(
        pattern.render_syntax(input),
        "plugin::(disable|enable|load)"
    );
    assert!(matches!(&pattern.parts[0].kind, PatternPart::Literal(_)));
    let PatternPart::Group { kind, patterns } = &pattern.parts[1].kind else {
        panic!("expected zsh bare group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable", "load"]
    );
}

#[test]
fn test_zsh_case_group_separator_after_parameter_segment() {
    let input = concat!(
        "case \"$mode\" in\n",
        "  (${kind}|literal)) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    let PatternPart::Group { kind, patterns } = &pattern.parts[0].kind else {
        panic!("expected zsh group with parameter branch");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["${kind}", "literal"]
    );
}

#[test]
fn test_zsh_case_group_separator_after_long_literal_prefix() {
    let long_prefix = "a".repeat(600);
    let input = format!(
        "case \"$mode\" in\n  ({}|literal)) print ok ;;\nesac\n",
        long_prefix
    );
    let script = Parser::with_dialect(&input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    let PatternPart::Group { kind, patterns } = &pattern.parts[0].kind else {
        panic!("expected long zsh case group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(patterns.len(), 2);
    assert_eq!(patterns[0].render_syntax(&input), long_prefix);
    assert_eq!(patterns[1].render_syntax(&input), "literal");
}

#[test]
fn test_zsh_case_accepts_numeric_range_pattern() {
    let input = concat!("case \"$jobspec\" in\n", "  <->) print ok ;;\n", "esac\n",);
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases[0].patterns[0].render_syntax(input), "<->");
}

#[test]
fn test_zsh_case_accepts_wrapper_alternatives_with_whitespace() {
    let input = concat!(
        "case $line in\n",
        "  (#* | <->..<->)\n",
        "    print -nP %F{blue}\n",
        "    ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0]
            .patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["#*", "<->..<->"]
    );
}

#[test]
fn test_zsh_case_accepts_start_group_with_suffix() {
    let input = concat!(
        "case \"$OSTYPE\" in\n",
        "  (darwin|freebsd)*) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(pattern.render_syntax(input), "(darwin|freebsd)*");
    assert!(matches!(
        &pattern.parts[0].kind,
        PatternPart::Group {
            kind: PatternGroupKind::ExactlyOne,
            ..
        }
    ));
    assert!(matches!(&pattern.parts[1].kind, PatternPart::AnyString));
}

#[test]
fn test_zsh_case_accepts_optional_suffix_group_after_literal_url() {
    let input = concat!(
        "case \"$url\" in\n",
        "  https://github.com/ohmyzsh/ohmyzsh(|.git)) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(
        pattern_part_slices(pattern, input),
        vec!["https://github.com/ohmyzsh/ohmyzsh", "(|.git)"]
    );
    let PatternPart::Group { kind, patterns } = &pattern.parts[1].kind else {
        panic!("expected optional suffix group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["", ".git"]
    );
}

#[test]
fn test_zsh_case_bare_group_requires_sh_glob_off() {
    let input = concat!(
        "case \"$url\" in\n",
        "  https://github.com/ohmyzsh/ohmyzsh(|.git)) print ok ;;\n",
        "esac\n",
    );

    let default_script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (default_compound, _) = expect_compound(&default_script.body[0]);
    let AstCompoundCommand::Case(default_command) = default_compound else {
        panic!("expected case command");
    };
    assert_eq!(default_command.cases[0].patterns.len(), 1);

    let sh_glob_script = parse_zsh_with_options(input, |options| {
        options.sh_glob = OptionValue::On;
    })
    .file;
    let (sh_glob_compound, _) = expect_compound(&sh_glob_script.body[0]);
    let AstCompoundCommand::Case(sh_glob_command) = sh_glob_compound else {
        panic!("expected case command");
    };
    assert_eq!(
        sh_glob_command.cases[0]
            .patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["https://github.com/ohmyzsh/ohmyzsh(", ".git)",]
    );
}

#[test]
fn test_zsh_case_ksh_glob_requires_option() {
    let input = concat!(
        "case $mode in\n",
        "  @(disable|enable)) print ok ;;\n",
        "esac\n",
    );

    let default_script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;
    let (default_compound, _) = expect_compound(&default_script.body[0]);
    let AstCompoundCommand::Case(default_command) = default_compound else {
        panic!("expected case command");
    };
    assert_eq!(default_command.cases[0].patterns.len(), 1);
    let default_pattern = &default_command.cases[0].patterns[0];
    assert_eq!(default_pattern.render_syntax(input), "@(disable|enable)");
    assert!(matches!(
        &default_pattern.parts[0].kind,
        PatternPart::Literal(_)
    ));
    let PatternPart::Group { kind, patterns } = &default_pattern.parts[1].kind else {
        panic!("expected bare zsh group after literal prefix");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable"]
    );

    let script = parse_zsh_with_options(input, |options| {
        options.ksh_glob = OptionValue::On;
    })
    .file;
    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };
    assert_eq!(command.cases[0].patterns.len(), 1);
    let pattern = &command.cases[0].patterns[0];
    let [part] = pattern.parts.as_slice() else {
        panic!("expected a single group part");
    };
    let PatternPart::Group { kind, patterns } = &part.kind else {
        panic!("expected ksh glob group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["disable", "enable"]
    );
}

#[test]
fn test_zsh_case_accepts_wrapper_alternatives_with_empty_first_pattern() {
    let input = concat!(
        "case ${ICE[proto]} in\n",
        "  (|ftp(|s)|git|http(|s)|rsync|ssh) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let rendered = command.cases[0]
        .patterns
        .iter()
        .map(|pattern| pattern.render_syntax(input))
        .collect::<Vec<_>>();
    assert_eq!(rendered.len(), 6);
    assert_eq!(rendered[0], "");
    assert!(rendered[1].starts_with("ftp"));
    assert!(rendered[1].ends_with("(|s)"));
    assert_eq!(rendered[2], "git");
    assert!(rendered[3].starts_with("http"));
    assert!(rendered[3].ends_with("(|s)"));
    assert_eq!(rendered[4], "rsync");
    assert_eq!(rendered[5], "ssh");
}

#[test]
fn test_zsh_case_accepts_infix_group_with_trailing_wildcard() {
    let input = concat!(
        "case $widgets[$widget] in\n",
        "  user:_zsh_autosuggest_(bound|orig)_*) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    let pattern = &command.cases[0].patterns[0];
    assert_eq!(
        pattern.render_syntax(input),
        "user:_zsh_autosuggest_(bound|orig)_*"
    );
    assert!(matches!(&pattern.parts[0].kind, PatternPart::Literal(_)));
    let PatternPart::Group { kind, patterns } = &pattern.parts[1].kind else {
        panic!("expected infix group");
    };
    assert_eq!(*kind, PatternGroupKind::ExactlyOne);
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.render_syntax(input))
            .collect::<Vec<_>>(),
        vec!["bound", "orig"]
    );
    assert!(matches!(&pattern.parts[2].kind, PatternPart::Literal(_)));
    assert!(matches!(&pattern.parts[3].kind, PatternPart::AnyString));
}

#[test]
fn test_zsh_case_accepts_mixed_jobspec_patterns() {
    let input = concat!(
        "case \"$jobspec\" in\n",
        "  <->) print number ;;\n",
        "  \"\"|%|+) print current ;;\n",
        "  -) print previous ;;\n",
        "  [?]*) print contains ;;\n",
        "  *) print prefix ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases[0].patterns[0].render_syntax(input), "<->");
    assert_eq!(command.cases[1].patterns.len(), 3);
    assert!(matches!(
        &command.cases[1].patterns[0].parts[0].kind,
        PatternPart::Word(word) if is_fully_quoted(word)
    ));
    assert_eq!(command.cases[1].patterns[1].render_syntax(input), "%");
    assert_eq!(command.cases[1].patterns[2].render_syntax(input), "+");
    assert_eq!(command.cases[2].patterns[0].render_syntax(input), "-");
    assert_eq!(command.cases[3].patterns[0].render_syntax(input), "[?]*");
    assert_eq!(command.cases[4].patterns[0].render_syntax(input), "*");
}

#[test]
fn test_zsh_case_accepts_wrapped_wildcard_suffix_patterns() {
    let input = concat!(
        "case $line in\n",
        "  (*# SKIP*) print skip ;;\n",
        "  (ok*# TODO*) print xpass ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0].patterns[0].render_syntax(input),
        "*# SKIP*"
    );
    assert_eq!(
        command.cases[1].patterns[0].render_syntax(input),
        "ok*# TODO*"
    );
}

#[test]
fn test_zsh_case_accepts_wrapper_quoted_pattern_with_same_line_body() {
    let input = concat!("case $arg in\n", "  ($'\\n') print ok ;;\n", "esac\n",);
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0].patterns[0].render_syntax(input),
        concat!("$'", "\n", "'")
    );
}

#[test]
fn test_zsh_case_preserves_semipipe_terminator() {
    let input = concat!(
        "case $2 in\n",
        "  cygwin_nt-10.0-i686) bin='cygwin32/bin' ;|\n",
        "  *) print ok ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(
        command.cases[0].terminator,
        CaseTerminator::ContinueMatching
    );
    assert_eq!(command.cases[0].terminator_span.unwrap().slice(input), ";|");
}

#[test]
fn test_case_preserves_bash_fallthrough_terminator_spans() {
    let input = concat!(
        "case $mode in\n",
        "  start) printf '%s\\n' start ;&\n",
        "  stop) printf '%s\\n' stop ;;&\n",
        "esac\n",
    );
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases[0].terminator, CaseTerminator::FallThrough);
    assert_eq!(command.cases[0].terminator_span.unwrap().slice(input), ";&");
    assert_eq!(command.cases[1].terminator, CaseTerminator::Continue);
    assert_eq!(
        command.cases[1].terminator_span.unwrap().slice(input),
        ";;&"
    );
}

#[test]
fn test_zsh_case_preserves_semipipe_terminator_across_repeated_arms() {
    let input = concat!(
        "case $2 in\n",
        "  cygwin_nt-10.0-i686)   bin='cygwin32/bin'  ;|\n",
        "  cygwin_nt-10.0-x86_64) bin='cygwin64/bin'  ;|\n",
        "  msys_nt-10.0-i686)     bin='msys32/usr/bin';|\n",
        "  msys_nt-10.0-x86_64)   bin='msys64/usr/bin';|\n",
        "  cygwin_nt-10.0-*)\n",
        "    tmp='/cygdrive/c/tmp'\n",
        "  ;|\n",
        "  msys_nt-10.0-*)\n",
        "    tmp='/c/tmp'\n",
        "    env='MSYSTEM=MSYS'\n",
        "    intro+='PATH=\"$PATH:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl\"'\n",
        "    ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert_eq!(command.cases.len(), 6);
    assert_eq!(
        command.cases[..5]
            .iter()
            .map(|case| case.terminator)
            .collect::<Vec<_>>(),
        vec![CaseTerminator::ContinueMatching; 5]
    );
    assert_eq!(command.cases[5].terminator, CaseTerminator::Break);
}

#[test]
fn test_non_zsh_dialects_reject_zsh_case_group_and_semipipe_forms() {
    let group_case = concat!(
        "case \"$OSTYPE\" in\n",
        "  (darwin|freebsd)*) print ok ;;\n",
        "esac\n",
    );
    let semipipe_case = concat!(
        "case $2 in\n",
        "  cygwin*) bin='cygwin32/bin' ;|\n",
        "  *) print ok ;;\n",
        "esac\n",
    );

    for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
        assert!(
            Parser::with_dialect(group_case, dialect).parse().is_err(),
            "expected {dialect:?} to reject zsh bare case groups",
        );
        assert!(
            Parser::with_dialect(semipipe_case, dialect)
                .parse()
                .is_err(),
            "expected {dialect:?} to reject zsh ;| case terminators",
        );
    }
}

#[test]
fn test_parse_zsh_case_arm_with_multiline_or_brace_fallback_group() {
    let input = concat!(
        "case \"${file:l}\" in\n",
        "  (*.tar.gz|*.tgz)\n",
        "    (( $+commands[pigz] )) && { tar -I pigz -xvf \"$full_path\" } || tar zxvf \"$full_path\" ;;\n",
        "  (*.tar.bz2|*.tbz|*.tbz2)\n",
        "    (( $+commands[pbzip2] )) && { tar -I pbzip2 -xvf \"$full_path\" } || tar xvjf \"$full_path\" ;;\n",
        "  (*.tar.xz|*.txz)\n",
        "    (( $+commands[pixz] )) && { tar -I pixz -xvf \"$full_path\" } || {\n",
        "      tar --xz --help &> /dev/null \\\n",
        "      && tar --xz -xvf \"$full_path\" \\\n",
        "      || xzcat \"$full_path\" | tar xvf -\n",
        "    } ;;\n",
        "esac\n",
    );
    let script = Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap()
        .file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Case(command) = compound else {
        panic!("expected case command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.cases.len(), 3);

    let fallback = expect_binary(&command.cases[2].body[0]);
    assert_eq!(fallback.op, BinaryOp::Or);
    let (group, group_redirects) = expect_compound(&fallback.right);
    let AstCompoundCommand::BraceGroup(body) = group else {
        panic!("expected multiline fallback brace group");
    };
    assert!(group_redirects.is_empty());
    assert_eq!(body.len(), 1);
    assert_eq!(expect_binary(&body[0]).op, BinaryOp::Or);
}

#[test]
fn test_parse_zsh_case_arm_with_dynamic_function_definition() {
    let input = "case $widget_type in\n  *)\n    if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n      _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n      zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n    else\n      print -r -- >&2 unhandled\n    fi\n    ;;\nesac\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_case_statement_with_eval_wrapped_widget_rebindings() {
    let input = "case ${widgets[$cur_widget]:-\"\"} in\n  user:_zsh_highlight_widget_*);;\n  user:*) zle -N $prefix-$cur_widget ${widgets[$cur_widget]#*:}\n          eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n          zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n  completion:*) zle -C $prefix-$cur_widget ${${(s.:.)widgets[$cur_widget]}[2,3]}\n                eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget ${(q)prefix}-${(q)cur_widget} -- \\\"\\$@\\\" }\"\n                zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n  builtin) eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n           zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n  *)\n     if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n       _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n       zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n     else\n       print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n       print -r -- >&2 \"zsh-syntax-highlighting: (This is sometimes caused by doing \\`bindkey <keys> ${(q-)cur_widget}\\` without creating the ${(qq)cur_widget} widget with \\`zle -N\\` or \\`zle -C\\`.)\"\n     fi\nesac\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_precmd_function_with_inline_case_arm_and_subshell_background() {
    let input = "_zshz_precmd() {\n  setopt LOCAL_OPTIONS UNSET\n  [[ $PWD == \"$HOME\" ]] || (( ZSHZ[DIRECTORY_REMOVED] )) && return\n\n  local exclude\n  for exclude in ${(@)ZSHZ_EXCLUDE_DIRS:-${(@)_Z_EXCLUDE_DIRS}}; do\n    case $PWD in\n      ${exclude}|${exclude}/*) return ;;\n    esac\n  done\n\n  if [[ $OSTYPE == (cygwin|msys) ]]; then\n    zshz --add \"$PWD\"\n  else\n    (zshz --add \"$PWD\" &)\n  fi\n\n  : $RANDOM\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_for_loop_case_arm_with_dynamic_function_definition() {
    let input = "for cur_widget in $widgets_to_bind; do\n  case ${widgets[$cur_widget]:-\"\"} in\n    *)\n       if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n         _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n         zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n       else\n         print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n       fi\n  esac\n done\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_function_case_with_builtin_eval_arm_before_dynamic_function_arm() {
    let input = "_zsh_highlight_bind_widgets()\n{\n  local cur_widget\n  for cur_widget in $widgets_to_bind; do\n    case ${widgets[$cur_widget]:-\"\"} in\n      builtin) eval \"_zsh_highlight_widget_${(q)prefix}-${(q)cur_widget}() { _zsh_highlight_call_widget .${(q)cur_widget} -- \\\"\\$@\\\" }\"\n               zle -N $cur_widget _zsh_highlight_widget_$prefix-$cur_widget;;\n      *)\n         if [[ $cur_widget == zle-* ]] && (( ! ${+widgets[$cur_widget]} )); then\n           _zsh_highlight_widget_${cur_widget}() { :; _zsh_highlight }\n           zle -N $cur_widget _zsh_highlight_widget_$cur_widget\n         else\n           print -r -- >&2 \"zsh-syntax-highlighting: unhandled ZLE widget ${(qq)cur_widget}\"\n         fi\n    esac\n  done\n}\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}
