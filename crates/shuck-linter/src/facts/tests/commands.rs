use super::*;

#[test]
fn summarizes_command_options_and_invokers() {
    let source = "#!/bin/bash\nread -r name\necho -ne hi\necho '-I' hi\necho \"\\\\n\"\necho \\x41\necho \"prefix $VAR \\\\0 suffix\"\ncommand echo \\n\nsed 's/foo/bar/'\nsed -e 's/foo/bar/'\nsed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\1:'\nsed 's/[]\\[^$.*/]/\\\\&/g'\nsed 's/\\([/&]\\)/\\\\\\1/g'\nsed -n 's/foo/bar/p'\nsed --expression 's/foo/bar/'\nsed -r 's/foo/bar/'\nsed \\\"s/foo/bar/\\\"\ntr -ds a-z A-Z\ntr -- 'a-z' xyz\nprintf -v out \"$fmt\" value\nprintf '%q\\n' foo\nprintf '%*q\\n' 10 bar\nunset -f curl other\nfind . -print0 | xargs -0 rm\nfind . -type d -name CVS | xargs -iX rm -rf X\nfind . -type d -name CVS | xargs --replace rm -rf {}\nfind . -name a -o -name b -print\nfind . -name *.cfg\nfind . -name \"$prefix\"*.jar\nfind . -wholename */tmp/*\nfind . -name \\*.ignore\nfind . -type f*\nrm -rf \"$dir\"/*\nrm -rf \"$dir\"/sub/*\nrm -rf \"$dir\"/lib\nrm -rf \"$dir\"/*.log\nrm -rf \"$rootdir/$md_type/$to\"\nrm -rf \"$configdir/all/retroarch/$dir\"\nrm -rf \"$md_inst/\"*\nwait -n\nwait -- -n\ngrep -o content file | wc -l\nexit foo\nset -eETo pipefail\nset euox pipefail\n./configure --with-optmizer=${CFLAGS}\nconfigure \"--enable-optmizer=${CFLAGS}\"\n./configure --with-optimizer=${CFLAGS}\nps -p 1 -o comm=\nps p 123 -o comm=\nps -ef\ndoas printf '%s\\n' hi\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let read = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("read"))
        .expect("expected read fact");
    assert_eq!(
        read.options().read().map(|read| read.uses_raw_input),
        Some(true)
    );

    let echo = facts
        .commands()
        .iter()
        .find(|fact| {
            fact.effective_name_is("echo")
                && fact
                    .options()
                    .echo()
                    .and_then(|echo| echo.portability_flag_word())
                    .is_some()
        })
        .and_then(|fact| fact.options().echo())
        .expect("expected echo facts");
    assert_eq!(
        echo.portability_flag_word()
            .map(|word| word.span.slice(source)),
        Some("-ne")
    );
    assert_eq!(
        facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("echo"))
            .nth(1)
            .and_then(|fact| fact.options().echo())
            .and_then(|echo| echo.portability_flag_word())
            .map(|word| word.span.slice(source)),
        None
    );
    assert_eq!(
        facts
            .echo_backslash_escape_word_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["\"\\\\n\"", "\\x41", "\"prefix $VAR \\\\0 suffix\""]
    );

    let sed_commands = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("sed"))
        .collect::<Vec<_>>();
    assert_eq!(sed_commands.len(), 9);
    assert!(
        sed_commands[0]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        sed_commands[1]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        sed_commands[2]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        !sed_commands[3]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        !sed_commands[4]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        !sed_commands[5]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        !sed_commands[6]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        !sed_commands[7]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );
    assert!(
        !sed_commands[8]
            .options()
            .sed()
            .is_some_and(|sed| sed.has_single_substitution_script())
    );

    let tr = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("tr") && fact.options().tr().is_some())
        .and_then(|fact| fact.options().tr())
        .expect("expected tr facts");
    assert_eq!(
        tr.operand_words()
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a-z", "A-Z"]
    );
    let quoted_tr = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("tr"))
        .nth(1)
        .and_then(|fact| fact.options().tr())
        .expect("expected second tr facts");
    assert_eq!(
        quoted_tr
            .operand_words()
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["'a-z'", "xyz"]
    );

    let printf = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("printf") && fact.options().printf().is_some())
        .expect("expected printf fact");
    assert_eq!(
        printf
            .options()
            .printf()
            .and_then(|printf| printf.format_word)
            .map(|word| word.span.slice(source)),
        Some("\"$fmt\"")
    );
    assert!(
        printf
            .options()
            .printf()
            .is_some_and(|printf| !printf.format_word_has_literal_percent)
    );
    assert!(
        !printf
            .options()
            .printf()
            .is_some_and(|printf| printf.uses_q_format)
    );

    let q_printf = facts
        .commands()
        .iter()
        .find(|fact| {
            fact.effective_name_is("printf")
                && fact
                    .options()
                    .printf()
                    .is_some_and(|printf| printf.uses_q_format)
        })
        .and_then(|fact| fact.options().printf())
        .expect("expected q printf facts");
    assert!(q_printf.uses_q_format);
    assert_eq!(
        q_printf.format_word.map(|word| word.span.slice(source)),
        Some("'%q\\n'")
    );

    let star_q_printf = facts
        .commands()
        .iter()
        .find(|fact| {
            fact.effective_name_is("printf")
                && fact
                    .options()
                    .printf()
                    .and_then(|printf| printf.format_word)
                    .map(|word| word.span.slice(source))
                    == Some("'%*q\\n'")
        })
        .and_then(|fact| fact.options().printf())
        .expect("expected star-width q printf facts");
    assert!(star_q_printf.uses_q_format);
    assert!(star_q_printf.format_word_has_literal_percent);

    let unset = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("unset"))
        .and_then(|fact| fact.options().unset())
        .expect("expected unset facts");
    assert!(unset.function_mode);
    assert!(unset.targets_function_name(source, "curl"));
    assert!(!unset.targets_function_name(source, "missing"));

    let find = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("find"))
        .and_then(|fact| fact.options().find())
        .expect("expected find facts");
    assert!(find.has_print0);
    let find_or_without_grouping_spans = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("find"))
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.or_without_grouping_spans().iter().copied())
        .map(|span| span.slice(source))
        .collect::<Vec<_>>();
    assert_eq!(find_or_without_grouping_spans, vec!["-print"]);
    let find_glob_pattern_operand_spans = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("find"))
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.glob_pattern_operand_spans().iter().copied())
        .map(|span| span.slice(source))
        .collect::<Vec<_>>();
    assert_eq!(
        find_glob_pattern_operand_spans,
        vec!["*.cfg", "\"$prefix\"*.jar", "*/tmp/*"]
    );

    let find_exec_shell = facts
        .commands()
        .iter()
        .filter(|fact| {
            fact.has_wrapper(WrapperKind::FindExec) || fact.has_wrapper(WrapperKind::FindExecDir)
        })
        .find_map(|fact| fact.options().find_exec_shell());
    assert!(
        find_exec_shell.is_none(),
        "fixture without a shell-backed find exec should not match"
    );

    let xargs = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("xargs"))
        .and_then(|fact| fact.options().xargs())
        .expect("expected xargs facts");
    assert!(xargs.uses_null_input);
    let inline_replace_option_spans = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("xargs"))
        .filter_map(|fact| fact.options().xargs())
        .flat_map(|xargs| xargs.inline_replace_option_spans().iter().copied())
        .map(|span| span.slice(source))
        .collect::<Vec<_>>();
    assert_eq!(inline_replace_option_spans, vec!["-iX"]);

    let wait = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("wait") && fact.options().wait().is_some())
        .and_then(|fact| fact.options().wait())
        .expect("expected wait facts");
    assert_eq!(
        wait.option_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["-n"]
    );

    let set = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("set"))
        .and_then(|fact| fact.options().set())
        .expect("expected set facts");
    assert_eq!(set.errexit_change, Some(true));
    assert_eq!(set.errtrace_change, Some(true));
    assert_eq!(set.functrace_change, Some(true));
    assert_eq!(set.pipefail_change, Some(true));
    assert_eq!(
        set.errtrace_flag_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["-eETo"]
    );
    assert_eq!(
        set.functrace_flag_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["-eETo"]
    );
    assert_eq!(
        set.pipefail_option_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["pipefail"]
    );
    assert_eq!(
        set.non_posix_option_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["pipefail"]
    );
    let set_without_prefix_spans = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("set"))
        .filter_map(|fact| fact.options().set())
        .flat_map(|set| set.flags_without_prefix_spans().iter().copied())
        .collect::<Vec<_>>();
    assert_eq!(
        set_without_prefix_spans
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["euox"]
    );
    let ps_pid_selector_flags = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("ps"))
        .filter_map(|fact| fact.options().ps().map(|ps| ps.has_pid_selector))
        .collect::<Vec<_>>();
    assert_eq!(ps_pid_selector_flags, vec![true, true, false]);
    let rm_spans = facts
        .commands()
        .iter()
        .filter_map(|fact| fact.options().rm())
        .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
        .collect::<Vec<_>>();
    assert_eq!(
        rm_spans
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec![
            "\"$dir\"/*",
            "\"$dir\"/lib",
            "\"$rootdir/$md_type/$to\"",
            "\"$md_inst/\"*"
        ]
    );
    let grep = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("grep"))
        .and_then(|fact| fact.options().grep())
        .expect("expected grep facts");
    assert!(grep.uses_only_matching);
    assert!(!grep.uses_fixed_strings);
    assert_eq!(
        grep.patterns()
            .iter()
            .map(|pattern| pattern.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["content"]
    );

    let exit = facts
        .commands()
        .iter()
        .find(|fact| fact.options().exit().is_some())
        .and_then(|fact| fact.options().exit())
        .expect("expected exit facts");
    assert_eq!(
        exit.status_word.map(|word| word.span.slice(source)),
        Some("foo")
    );
    assert!(exit.has_static_status());
    assert!(!exit.is_numeric_literal);
    assert!(exit.has_invalid_status_argument());

    let doas = facts
        .commands()
        .iter()
        .find(|fact| fact.has_wrapper(WrapperKind::SudoFamily))
        .and_then(|fact| fact.options().sudo_family())
        .expect("expected sudo-family facts");
    assert_eq!(doas.invoker, SudoFamilyInvoker::Doas);
}

#[test]
fn bracket_command_name_separator_fact_tracks_split_test_commands() {
    let source = "\
#!/bin/sh
amoeba=\"\" [ \"${AMOEBA:-yes}\" = \"yes\" ]
>out [ \"$1\" = yes ]
[
  \"$1\" = yes
]
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let line2 = facts
        .commands()
        .iter()
        .find(|fact| fact.literal_name() == Some("[") && fact.span().start.line == 2)
        .expect("expected inline test command");
    let line3 = facts
        .commands()
        .iter()
        .find(|fact| fact.literal_name() == Some("[") && fact.span().start.line == 3)
        .expect("expected redirected bracket command");
    let line4 = facts
        .commands()
        .iter()
        .find(|fact| fact.literal_name() == Some("[") && fact.span().start.line == 4)
        .expect("expected plain bracket command");

    assert!(line2.bracket_command_name_needs_separator(source));
    assert!(!line3.bracket_command_name_needs_separator(source));
    assert!(!line4.bracket_command_name_needs_separator(source));
}

#[test]
fn set_command_facts_track_non_posix_set_o_options() {
    let source = "\
#!/bin/sh
set -o pipefail
set +o posix
set -eo emacs
set -o bogus -- bogus
set -o vi
set -o allexport
set -o \"$mode\"
set -- -o posix
";

    with_facts(source, None, |_, facts| {
        let spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("set"))
            .filter_map(|fact| fact.options().set())
            .flat_map(|set| set.non_posix_option_spans().iter().copied())
            .collect::<Vec<_>>();

        assert_eq!(
            spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["pipefail", "posix", "emacs", "bogus"]
        );
    });
}

#[test]
fn command_facts_surface_command_name_shape_helpers() {
    let source = "\
#!/bin/sh
\"$root/pkg/{{name}}/bin/{{cmd}}\"
\"ERROR: missing first arg for name to docker_compose_version_test()\"
\"${loader:?}\"
\"${cmd:-\\}}\"
\"${cmd:-`printf '}'`}\"
\"$(printf cmd)\"
\"$(printf ')')\"
\"$(echo `printf ')'`)\"
printf#
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let command = |text: &str| {
        facts
            .commands()
            .iter()
            .find(|command| {
                command
                    .body_word_span()
                    .is_some_and(|span| span.slice(source) == text)
            })
            .expect("expected command fact")
    };

    let placeholder = command("\"$root/pkg/{{name}}/bin/{{cmd}}\"");
    let suspicious_quote =
        command("\"ERROR: missing first arg for name to docker_compose_version_test()\"");
    let parameter_expansion = command("\"${loader:?}\"");
    let escaped_brace_parameter_expansion = command("\"${cmd:-\\}}\"");
    let backtick_brace_parameter_expansion = command("\"${cmd:-`printf '}'`}\"");
    let command_substitution = command("\"$(printf cmd)\"");
    let quoted_paren_command_substitution = command("\"$(printf ')')\"");
    let backtick_paren_command_substitution = command("\"$(echo `printf ')'`)\"");
    let hash_suffix = command("printf#");

    assert!(placeholder.body_word_contains_template_placeholder(source));
    assert!(suspicious_quote.body_word_has_suspicious_quoted_command_trailer(source, None));
    assert!(!parameter_expansion.body_word_has_suspicious_quoted_command_trailer(source, None));
    assert!(
        !escaped_brace_parameter_expansion
            .body_word_has_suspicious_quoted_command_trailer(source, None)
    );
    assert!(
        !backtick_brace_parameter_expansion
            .body_word_has_suspicious_quoted_command_trailer(source, None)
    );
    assert!(!command_substitution.body_word_has_suspicious_quoted_command_trailer(source, None));
    assert!(
        !quoted_paren_command_substitution
            .body_word_has_suspicious_quoted_command_trailer(source, None)
    );
    assert!(
        !backtick_paren_command_substitution
            .body_word_has_suspicious_quoted_command_trailer(source, None)
    );
    assert!(hash_suffix.body_word_has_hash_suffix(source));
}

#[test]
fn ssh_command_facts_match_shellcheck_command_shape() {
    let source = "\
#!/bin/bash
ssh host \"echo $HOME\"
\\ssh host \"echo $USER\"
ssh -i key host \"echo $PATH\"
ssh host -t \"echo $PWD\"
ssh host ls -l \"$TMPDIR\"
ssh host cmd \"--flag\" \"$HOME\"
ssh host cmd '-t' \"$USER\"
";

    with_facts(source, None, |_, facts| {
        let ssh_commands = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("ssh") && fact.wrappers().is_empty())
            .collect::<Vec<_>>();

        assert_eq!(ssh_commands.len(), 7);
        assert_eq!(
            ssh_commands[0]
                .options()
                .ssh()
                .map(|ssh| ssh.local_expansion_spans().len()),
            Some(1)
        );
        assert_eq!(
            ssh_commands[1]
                .options()
                .ssh()
                .map(|ssh| ssh.local_expansion_spans().len()),
            Some(1)
        );
        assert!(ssh_commands[2].options().ssh().is_none());
        assert!(ssh_commands[3].options().ssh().is_none());
        assert!(ssh_commands[4].options().ssh().is_none());
        assert!(ssh_commands[5].options().ssh().is_none());
        assert!(ssh_commands[6].options().ssh().is_none());
    });
}

#[test]
fn summarizes_echo_options_for_path_qualified_echo() {
    let source = "#!/bin/sh\nvalue=$(/usr/ucb/echo -n hi)\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let echo = facts
        .commands()
        .iter()
        .find(|fact| fact.literal_name() == Some("/usr/ucb/echo"))
        .and_then(|fact| fact.options().echo())
        .expect("expected path-qualified echo facts");

    assert_eq!(
        echo.portability_flag_word()
            .map(|word| word.span.slice(source)),
        Some("-n")
    );
}

#[test]
fn builds_tr_facts_inside_escaped_quoted_command_substitutions() {
    let source = "#!/bin/sh\necho -n \"\\\"adp_$(echo $var | tr A-Z a-z)\\\": [\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let tr = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("tr"))
        .and_then(|fact| fact.options().tr())
        .expect("expected escaped quoted command substitution tr facts");
    assert_eq!(
        tr.operand_words()
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["A-Z", "a-z"]
    );
}

#[test]
fn builds_tr_facts_inside_piped_command_substitutions_with_quoted_operands() {
    let source = "#!/bin/sh\nATLAS_SHARED=$(echo \"$ATLAS_SHARED\"|cut -b 1|tr a-z A-Z)\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let tr = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("tr"))
        .expect("expected tr command fact");
    assert!(tr.wrappers().is_empty());
    let tr_options = tr.options().tr().expect("expected tr options");
    assert_eq!(
        tr_options
            .operand_words()
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["a-z", "A-Z"]
    );
}

#[test]
fn tracks_quote_like_echo_escapes_inside_double_quotes_from_syntax_text() {
    let source = "#!/bin/bash\necho \"  echo Remember to run \\\\\\`updatedb\\\\'.\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert_eq!(
        facts
            .echo_backslash_escape_word_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["\"  echo Remember to run \\\\\\`updatedb\\\\'.\""]
    );
}

#[test]
fn tracks_echo_backslash_double_quote_escape_shapes() {
    let source = "#!/bin/bash\necho -DLATEX=\\\\\"$(which latex)\\\\\"\necho \"  .TargetPath = \\\"\\\\\\\\host.lan\\\\Data\\\"\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert_eq!(
        facts
            .echo_backslash_escape_word_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec![
            "-DLATEX=\\\\\"$(which latex)\\\\\"",
            "\"  .TargetPath = \\\"\\\\\\\\host.lan\\\\Data\\\"\""
        ]
    );
}

#[test]
fn ignores_json_like_backslash_quote_wrappers_around_variables() {
    let source = "#!/bin/bash\necho \"LABEL com.dokku.docker-image-labeler/alternate-tags=[\\\\\\\"$DOCKER_IMAGE\\\\\\\"]\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert!(facts.echo_backslash_escape_word_spans().is_empty());
}

#[test]
fn ignores_quote_plumbing_around_adjacent_quoted_fragments() {
    let source = "\
#!/bin/bash
echo \"$1\"=\\\"\"${PWD}\"\\\"
echo pin=\\'\"${new_pinned[*]}\"\\'
echo 'set -gx PATH '\\''\"${PYENV_ROOT}/shims\"\\'' $PATH'
echo SHOBJ_CC=\\'\"$SHOBJ_CC\"\\'
echo aws cloudwatch put-metric-alarm --alarm-actions \\'\"$ALARMACTION\"\\' --output=json
echo \"Rule \"\\#$COUNT
echo Saved to \\\"\"$FILENAME\"\\\" \\(\"$(du -h \"$OUTPUT\" | cut -f1)\"\\)
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert!(facts.echo_backslash_escape_word_spans().is_empty());
}

#[test]
fn tracks_find_print0_without_treating_it_as_formatted_output() {
    let source = "\
#!/bin/bash
find . -print0 | xargs rm
find . -printf '%h\\n' | xargs mv -t dest
find . -print | xargs rm
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let find_facts = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("find"))
        .filter_map(|fact| fact.options().find())
        .collect::<Vec<_>>();

    assert_eq!(find_facts.len(), 3);
    assert!(find_facts[0].has_print0);
    assert!(!find_facts[0].has_formatted_output_action());
    assert!(find_facts[1].has_formatted_output_action());
    assert!(!find_facts[2].has_formatted_output_action());
}

#[test]
fn tracks_expr_string_helper_kinds_and_spans() {
    let source = "\
#!/bin/sh
expr length \"$mode\"
expr index \"$mode\" w
expr match \"$mode\" 'w'
expr substr \"$mode\" 1 1
expr \"$a\" = \"$b\"
expr 1 + 2
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let exprs = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("expr"))
        .filter_map(|fact| fact.options().expr())
        .collect::<Vec<_>>();

    assert_eq!(
        exprs
            .iter()
            .map(|expr| expr.string_helper_kind())
            .collect::<Vec<_>>(),
        vec![
            Some(ExprStringHelperKind::Length),
            Some(ExprStringHelperKind::Index),
            Some(ExprStringHelperKind::Match),
            Some(ExprStringHelperKind::Substr),
            None,
            None,
        ]
    );
    assert_eq!(
        exprs
            .iter()
            .map(|expr| expr.string_helper_span().map(|span| span.slice(source)))
            .collect::<Vec<_>>(),
        vec![
            Some("length"),
            Some("index"),
            Some("match"),
            Some("substr"),
            None,
            None,
        ]
    );
    assert!(exprs[3].uses_substr_string_form());
    assert_eq!(
        exprs
            .iter()
            .map(|expr| expr.uses_arithmetic_operator())
            .collect::<Vec<_>>(),
        vec![false, false, false, false, false, true]
    );
}

#[test]
fn tracks_printf_formats_with_and_without_literal_percents() {
    let source = "printf \"$fmt\" value\nprintf \"${left}${right}\" value\nprintf \"${fmt:-%s}\" value\nprintf \"$(echo %s)\" value\nprintf \"pre$foo\" value\nprintf \"%${width}s\\n\" value\nprintf \"${color}%s${reset}\" value\nprintf \"$fmt%s\" value\nprintf '%s\\n' value\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let printfs = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("printf"))
        .filter_map(|fact| fact.options().printf())
        .map(|printf| {
            (
                printf
                    .format_word
                    .map(|word| word.span.slice(source))
                    .expect("expected format word"),
                printf.format_word_has_literal_percent,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        printfs,
        vec![
            ("\"$fmt\"", false),
            ("\"${left}${right}\"", false),
            ("\"${fmt:-%s}\"", false),
            ("\"$(echo %s)\"", false),
            ("\"pre$foo\"", false),
            ("\"%${width}s\\n\"", true),
            ("\"${color}%s${reset}\"", true),
            ("\"$fmt%s\"", true),
            ("'%s\\n'", true),
        ]
    );
}

#[test]
fn builds_echo_to_sed_substitution_spans() {
    let source = "\
#!/bin/bash
echo $value | sed 's/foo/bar/'
echo \"$value\" | sed 's/foo/bar/g'
echo \"$value\" | sed 's§foo§bar§'
echo ${items[@]} | sed -e 's/foo/bar/2'
result=$(echo \"$(printf %s foo)\" | sed 's/foo/bar/')
COMMAND=$(echo \"$COMMAND\" | sed \"s#\\(--appendconfig *[^ $]*\\)#\\1'|'$conf#\")
RUNTIME=$(echo $OUT | sed \"s|$OUT|\\$this_dir|g\")
escaped_hostname=$(echo \"$hostname\" | sed 's/[]\\[\\.^$*+?{}()|]/\\\\&/g')
value=$(sed 's/[\\.|$(){}?+*^]/\\\\&/g' <<<\"$value\")
echo \"$value\" | sed 's/a/b/' <<<\"$shadow\"
CFLAGS=\"`echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS//\\\"`\"
OPTFLAG=\"`echo \\\"$CFLAGS\\\" | sed 's/^.*\\(-O[^ ]\\).*$/\\1/'`\"
EC2_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\\\1:'`\"
ESCAPED_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e \\\"s/foo/bar/\\\"`\"
FLAGGED_REGION=\"`echo \\\"$EC2_REGION\\\" | sed 's/foo/bar/g'`\"
echo \"$caps_add\" | sed 's/^/  /' \t
trimmed=$(sed 's/[[:space:]]*$//' <<<\"$value\")
literal=$(sed 's/[[:space:]]*$//' <<<literal)
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .echo_to_sed_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "echo $value | sed 's/foo/bar/'",
                "echo \"$value\" | sed 's/foo/bar/g'",
                "echo \"$value\" | sed 's§foo§bar§'",
                "echo ${items[@]} | sed -e 's/foo/bar/2'",
                "echo \"$(printf %s foo)\" | sed 's/foo/bar/'",
                "echo \"$COMMAND\" | sed \"s#\\(--appendconfig *[^ $]*\\)#\\1'|'$conf#\"",
                "echo $OUT | sed \"s|$OUT|\\$this_dir|g\"",
                "echo \"$hostname\" | sed 's/[]\\[\\.^$*+?{}()|]/\\\\&/g'",
                "sed 's/[\\.|$(){}?+*^]/\\\\&/g' <<<\"$value\"",
                "echo \"$value\" | sed 's/a/b/' <<<\"$shadow\"",
                "echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS",
                "echo \\\"$CFLAGS\\\" | sed 's/^.*\\(-O[^ ]\\).*$/\\1",
                "echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\",
                "echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e \\\"s/foo/bar",
                "echo \\\"$EC2_REGION\\\" | sed 's/foo/bar/",
                "echo \"$caps_add\" | sed 's/^/  /' \t",
                "sed 's/[[:space:]]*$//' <<<\"$value\"",
                "sed 's/[[:space:]]*$//' <<<literal",
            ]
        );
    });
}

#[test]
fn skips_nonmatching_echo_to_sed_substitution_shapes() {
    let source = "\
#!/bin/bash
echo literal | sed 's/foo/bar/'
echo prefix${value}suffix | sed 's/foo/bar/'
echo \"prefix${value}\" | sed 's/foo/bar/'
echo $left $right | sed 's/foo/bar/'
echo \"$left $right\" | sed 's/foo/bar/'
echo -n $value | sed 's/foo/bar/'
echo $value | sed -n 's/foo/bar/p'
echo $value | sed --expression 's/foo/bar/'
echo $value | sed -es/foo/bar/
echo $value | sed 's/foo/bar/' | cat
echo \"$ENDPOINT\" | sed 's/[:\\/]/_/g'
echo \"$PAYLOAD\" | sed 's/\\//-/g'
echo $PACKAGE_NAME | sed 's/\\./\\//g'
echo \"$value\" | sed \\\"s/foo/bar/\\\"
echo \"$key\" | sed 's/[]\\[^$.*/]/\\\\&/g'
echo \"${ENTRY}\" | sed 's/\\([/&]\\)/\\\\\\1/g'
sed 's/[]\\[^$.*/]/\\\\&/g' <<<\"$key\"
sed 's/\\([/&]\\)/\\\\\\1/g' <<<\"${ENTRY}\"
printf '%s\\n' \"$value\" | sed 's/foo/bar/'
echo \"prefix$(printf %s foo)\" | sed 's/foo/bar/'
";

    with_facts(source, None, |_, facts| {
        assert!(facts.echo_to_sed_substitution_spans().is_empty());
    });
}

#[test]
fn backtick_echo_to_sed_substitution_matches_shellcheck_columns_for_escaped_dollar_patterns() {
    let source = "\
#!/bin/bash
EC2_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\\\1:'`\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts.echo_to_sed_substitution_spans();
        assert_eq!(spans.len(), 1);
        let span = spans[0];
        assert_eq!(span.start.line, 2);
        assert_eq!(span.start.column, 14);
        assert_eq!(span.end.line, 2);
        assert_eq!(span.end.column, 76);
    });
}

#[test]
fn backtick_echo_to_sed_substitution_keeps_utf8_trim_offsets_on_char_boundaries() {
    let source = "\
#!/bin/bash
A=\"`echo \\\"$A\\\" | sed 's/foo\\\\$/é/'`\"
";

    with_facts(source, None, |_, facts| {
        let spans = facts.echo_to_sed_substitution_spans();
        assert_eq!(spans.len(), 1);
        let span = spans[0];
        assert_eq!(span.start.line, 2);
        assert_eq!(span.start.column, 5);
        assert_eq!(span.end.line, 2);
        assert_eq!(span.end.column, 33);
        assert_eq!(span.slice(source), "echo \\\"$A\\\" | sed 's/foo\\\\$/");
    });
}

#[test]
fn preserves_dynamic_unset_operands_after_option_parsing_stops() {
    let source = "\
#!/bin/bash
declare -A parts
key=one
unset parts[\"$key\"] extra
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let unset = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("unset"))
        .and_then(|fact| fact.options().unset())
        .expect("expected unset facts");

    assert_eq!(
        unset
            .operand_words()
            .iter()
            .map(|word| word.span.slice(source))
            .collect::<Vec<_>>(),
        vec!["parts[\"$key\"]", "extra"]
    );
}

#[test]
fn parses_unset_nameref_mode_separately_from_variable_mode() {
    let source = "\
#!/bin/bash
unset -n ref
unset -v value
unset -xn unknown
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let unsets = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("unset"))
        .filter_map(|fact| fact.options().unset())
        .collect::<Vec<_>>();

    assert_eq!(unsets.len(), 3);
    assert!(unsets[0].nameref_mode());
    assert!(!unsets[0].function_mode);
    assert!(unsets[0].options_parseable());
    assert!(!unsets[1].nameref_mode());
    assert!(unsets[1].options_parseable());
    assert!(unsets[2].nameref_mode());
    assert!(!unsets[2].options_parseable());
}

#[test]
fn records_unset_array_subscript_details_in_operand_facts() {
    let source = "\
#!/bin/bash
declare -A parts
declare -a nums
key=one
unset parts[\"$key\"] plain \"parts[safe]\" 'parts[also_safe]' nums[1]
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let unset = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("unset"))
        .and_then(|fact| fact.options().unset())
        .expect("expected unset facts");

    let operand_subscripts = unset
        .operand_facts()
        .iter()
        .map(|operand| {
            operand
                .array_subscript()
                .map(|_| operand.word().span.slice(source))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        operand_subscripts,
        vec![Some("parts[\"$key\"]"), None, None, None, Some("nums[1]"),]
    );
}

#[test]
fn precomputes_command_containment_and_barrier_flags_for_nested_commands() {
    let source = "\
#!/bin/bash
if init_if; then
  { brace_inner; }
fi
left && right
time timed
";

    with_facts(source, None, |_, facts| {
        let init_if = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("init_if"))
            .expect("expected init_if command");
        let brace_inner = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("brace_inner"))
            .expect("expected brace_inner command");
        let timed = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("timed"))
            .expect("expected timed command");
        let binary = facts
            .commands()
            .iter()
            .find(|fact| matches!(fact.command(), shuck_ast::Command::Binary(_)))
            .expect("expected binary command");

        let init_parent = facts
            .command_parent(init_if.id())
            .expect("expected if parent");
        assert!(matches!(
            init_parent.command(),
            shuck_ast::Command::Compound(shuck_ast::CompoundCommand::If(_))
        ));
        assert!(facts.command_is_dominance_barrier(init_parent.id()));

        let brace_parent = facts
            .command_parent(brace_inner.id())
            .expect("expected brace-group parent");
        assert!(matches!(
            brace_parent.command(),
            shuck_ast::Command::Compound(shuck_ast::CompoundCommand::BraceGroup(_))
        ));
        assert!(!facts.command_is_dominance_barrier(brace_parent.id()));

        let timed_parent = facts
            .command_parent(timed.id())
            .expect("expected time parent");
        assert!(matches!(
            timed_parent.command(),
            shuck_ast::Command::Compound(shuck_ast::CompoundCommand::Time(_))
        ));
        assert!(!facts.command_is_dominance_barrier(timed_parent.id()));
        assert!(facts.command_is_dominance_barrier(binary.id()));

        let brace_offset = source.find("brace_inner").expect("brace_inner offset");
        let right_offset = source.find("right").expect("right offset");
        assert_eq!(
            facts
                .innermost_command_at(brace_offset)
                .and_then(|fact| fact.effective_name()),
            Some("brace_inner")
        );
        assert_eq!(
            facts
                .innermost_command_at(right_offset)
                .and_then(|fact| fact.effective_name()),
            Some("right")
        );
    });
}

#[test]
fn collects_prefix_match_spans_from_unset_operands() {
    let source = "\
#!/bin/sh
unset -v \"${!prefix_@}\" x${!prefix_*} \"${!name}\" \"${!arr[@]}\"
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Sh);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let unset = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("unset"))
        .and_then(|fact| fact.options().unset())
        .expect("expected unset facts");

    assert_eq!(
        unset
            .prefix_match_operand_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["${!prefix_@}", "${!prefix_*}"]
    );
}

#[test]
fn tracks_mapfile_input_fd_and_grouped_find_or_branches() {
    let source = "#!/bin/bash\nmapfile -u 3 -t files 3< <(printf '%s\\n' hi)\nmapfile -C cb -c 1 lines\nfind . \\( -name a -o -name b -print \\)\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let mapfile = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("mapfile"))
        .and_then(|fact| fact.options().mapfile())
        .expect("expected mapfile facts");
    assert_eq!(mapfile.input_fd(), Some(3));
    assert_eq!(
        mapfile
            .target_name_uses()
            .iter()
            .map(|target| target.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["files"]
    );

    let callback_mapfile = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("mapfile"))
        .nth(1)
        .and_then(|fact| fact.options().mapfile())
        .expect("expected callback mapfile facts");
    assert_eq!(
        callback_mapfile
            .target_name_uses()
            .iter()
            .map(|target| target.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["lines"]
    );

    let dynamic_source = "#!/bin/bash\nmapfile -u \"$fd\" -t files < <(printf '%s\\n' hi)\n";
    let dynamic_output = Parser::new(dynamic_source).parse().unwrap();
    let dynamic_indexer = Indexer::new(dynamic_source, &dynamic_output);
    let dynamic_semantic =
        SemanticModel::build(&dynamic_output.file, dynamic_source, &dynamic_indexer);
    let dynamic_file_context = classify_file_context(dynamic_source, None, ShellDialect::Bash);
    let dynamic_facts = LinterFacts::build(
        &dynamic_output.file,
        dynamic_source,
        &dynamic_semantic,
        &dynamic_indexer,
        &dynamic_file_context,
    );

    let dynamic_mapfile = dynamic_facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("mapfile"))
        .and_then(|fact| fact.options().mapfile())
        .expect("expected dynamic mapfile facts");
    assert_eq!(dynamic_mapfile.input_fd(), None);

    let find_or_without_grouping_spans = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("find"))
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.or_without_grouping_spans().iter().copied())
        .map(|span| span.slice(source))
        .collect::<Vec<_>>();
    assert_eq!(find_or_without_grouping_spans, vec!["-print"]);
}

#[test]
fn tracks_find_exec_or_branches_without_action_only_false_positives() {
    let source = "\
#!/bin/bash
find . -name a -o -name b -exec rm -f {} \\;
find . -name a -o -name b -o -name c -exec cp {} out \\;
find . \\( -name a -o \\( -name b -exec rm -f {} \\; \\) \\) -print
find . -type l -o -print
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let find_or_without_grouping_spans = facts
        .commands()
        .iter()
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.or_without_grouping_spans().iter().copied())
        .map(|span| span.slice(source))
        .collect::<Vec<_>>();

    assert_eq!(find_or_without_grouping_spans, vec!["-exec", "-exec"]);
}

#[test]
fn parses_grep_pattern_words_from_flags_and_operands() {
    let source = "\
#!/bin/bash
grep item,[0-4] data.txt
grep -e item* data.txt
grep -eitem* data.txt
grep -oe item* data.txt
grep --regexp='a[b]c' data.txt
grep --regexp item? data.txt
grep --regexp=foo* data.txt
grep -eo item* data.txt
grep -F -- item* data.txt
grep -f patterns.txt item* data.txt
grep -F -E foo*bar data.txt
grep -E -F foo*bar data.txt
grep --exclude '*.txt' foo* data.txt
grep --label stdin foo* data.txt
grep --color foo* data.txt
grep --context 3 foo* data.txt
grep --regexp='*start' data.txt
grep -e'*start' data.txt
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let grep_patterns = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .map(|grep| {
            (
                grep.patterns()
                    .iter()
                    .map(|pattern| {
                        (
                            pattern.span().slice(source),
                            pattern.static_text(),
                            pattern.source_kind(),
                        )
                    })
                    .collect::<Vec<_>>(),
                grep.uses_fixed_strings,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        grep_patterns,
        vec![
            (
                vec![(
                    "item,[0-4]",
                    Some("item,[0-4]"),
                    GrepPatternSourceKind::ImplicitOperand,
                )],
                false,
            ),
            (
                vec![(
                    "item*",
                    Some("item*"),
                    GrepPatternSourceKind::ShortOptionSeparate,
                )],
                false,
            ),
            (
                vec![(
                    "-eitem*",
                    Some("item*"),
                    GrepPatternSourceKind::ShortOptionAttached,
                )],
                false,
            ),
            (
                vec![(
                    "item*",
                    Some("item*"),
                    GrepPatternSourceKind::ShortOptionSeparate,
                )],
                false,
            ),
            (
                vec![(
                    "--regexp='a[b]c'",
                    Some("a[b]c"),
                    GrepPatternSourceKind::LongOptionAttached,
                )],
                false,
            ),
            (
                vec![(
                    "item?",
                    Some("item?"),
                    GrepPatternSourceKind::LongOptionSeparate,
                )],
                false,
            ),
            (
                vec![(
                    "--regexp=foo*",
                    Some("foo*"),
                    GrepPatternSourceKind::LongOptionAttached,
                )],
                false,
            ),
            (
                vec![("-eo", Some("o"), GrepPatternSourceKind::ShortOptionAttached,)],
                false,
            ),
            (
                vec![(
                    "item*",
                    Some("item*"),
                    GrepPatternSourceKind::ImplicitOperand,
                )],
                true,
            ),
            (Vec::new(), false),
            (
                vec![(
                    "foo*bar",
                    Some("foo*bar"),
                    GrepPatternSourceKind::ImplicitOperand,
                )],
                false,
            ),
            (
                vec![(
                    "foo*bar",
                    Some("foo*bar"),
                    GrepPatternSourceKind::ImplicitOperand,
                )],
                true,
            ),
            (
                vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                false,
            ),
            (
                vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                false,
            ),
            (
                vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                false,
            ),
            (
                vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                false,
            ),
            (
                vec![(
                    "--regexp='*start'",
                    Some("*start"),
                    GrepPatternSourceKind::LongOptionAttached,
                )],
                false,
            ),
            (
                vec![(
                    "-e'*start'",
                    Some("*start"),
                    GrepPatternSourceKind::ShortOptionAttached,
                )],
                false,
            ),
        ]
    );
}

#[test]
fn grep_pattern_facts_track_leading_glob_style_star_prefixes() {
    let source = "\
#!/bin/bash
grep '*start' data.txt
grep ''*user data.txt
grep '^*' data.txt
grep '^*foo' data.txt
grep --regexp='*start' data.txt
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let grep_patterns = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.patterns().iter())
        .map(|pattern| {
            (
                pattern.span().slice(source),
                pattern.static_text(),
                pattern.starts_with_glob_style_star(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        grep_patterns,
        vec![
            ("'*start'", Some("*start"), true),
            ("''*user", Some("*user"), true),
            ("'^*'", Some("^*"), true),
            ("'^*foo'", Some("^*foo"), false),
            ("--regexp='*start'", Some("*start"), true),
        ]
    );
}

#[test]
fn grep_pattern_facts_track_shellcheck_style_pattern_position() {
    let source = "\
#!/bin/bash
grep -e '*first' -e '*second' data.txt
grep -m 1 '*implicit-after-option' data.txt
grep -m1 '*implicit-after-attached-option' data.txt
grep -A 1 -e '*explicit-after-option' data.txt
grep -e '*explicit-before-option' -A 1 data.txt
grep -$dynamic_option 1 -e '*explicit-after-dynamic-option' data.txt
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let grep_patterns = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.patterns().iter())
        .map(|pattern| {
            (
                pattern.span().slice(source),
                pattern.is_first_pattern(),
                pattern.follows_separate_option_argument(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        grep_patterns,
        vec![
            ("'*first'", true, false),
            ("'*second'", false, false),
            ("'*implicit-after-option'", true, true),
            ("'*implicit-after-attached-option'", true, false),
            ("'*explicit-after-option'", true, true),
            ("'*explicit-before-option'", true, false),
            ("'*explicit-after-dynamic-option'", true, true),
        ]
    );
}

#[test]
fn grep_pattern_facts_track_glob_style_star_confusion() {
    let source = "\
#!/bin/bash
grep start* data.txt
grep 'foo*bar' data.txt
grep 'foo\\*bar*' data.txt
grep '^#* OPTIONS #*$' data.txt
grep -Eo 'https?://[[:alnum:]./?&!$#%@*;:+~_=-]+' data.txt
grep '^root:[:!*]' data.txt
grep -e 'Swarm:*\\sactive\\s*' data.txt
grep 'foo*bar+' data.txt
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let grep_patterns = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.patterns().iter())
        .map(|pattern| {
            (
                pattern.span().slice(source),
                pattern.has_glob_style_star_confusion(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        grep_patterns,
        vec![
            ("start*", true),
            ("'foo*bar'", true),
            ("'foo\\*bar*'", true),
            ("'^#* OPTIONS #*$'", false),
            ("'https?://[[:alnum:]./?&!$#%@*;:+~_=-]+'", false),
            ("'^root:[:!*]'", false),
            ("'Swarm:*\\sactive\\s*'", false),
            ("'foo*bar+'", false),
        ]
    );
}

#[test]
fn grep_pattern_facts_track_glob_style_star_replacement_spans() {
    let source = "\
#!/bin/bash
grep 'foo\\*bar*' data.txt
grep item\\* data.txt
grep start* data.txt
grep --regexp='start*' data.txt
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let grep_patterns = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.patterns().iter())
        .map(|pattern| {
            (
                pattern.span().slice(source),
                pattern
                    .glob_style_star_replacement_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        grep_patterns,
        vec![
            ("'foo\\*bar*'", vec!["*"]),
            ("item\\*", vec!["\\*"]),
            ("start*", vec!["*"]),
            ("--regexp='start*'", vec!["*"]),
        ]
    );
}

#[test]
fn attached_short_e_patterns_do_not_accidentally_toggle_only_matching() {
    let source = "\
#!/bin/bash
grep -oe item* data.txt
grep -eo item* data.txt
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let grep_modes = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .map(|grep| grep.uses_only_matching)
        .collect::<Vec<_>>();

    assert_eq!(grep_modes, vec![true, false]);
}

#[test]
fn tracks_dynamic_ps_pid_selectors() {
    let source = "\
#!/bin/bash
ps -p\"$pid\" -o comm=
ps --pid=\"$pid\" -o comm=
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let ps_commands = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("ps"))
        .collect::<Vec<_>>();

    assert_eq!(ps_commands.len(), 2);
    assert!(
        ps_commands
            .iter()
            .all(|fact| fact.options().ps().is_some_and(|ps| ps.has_pid_selector))
    );
}

#[test]
fn tracks_bare_ps_pid_operands() {
    let source = "\
#!/bin/bash
ps 1 -o comm=
ps 1,2 -o comm=
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let ps_commands = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("ps"))
        .collect::<Vec<_>>();

    assert_eq!(ps_commands.len(), 2);
    assert!(
        ps_commands
            .iter()
            .all(|fact| fact.options().ps().is_some_and(|ps| ps.has_pid_selector))
    );
}

#[test]
fn tracks_ps_pid_selectors_after_bsd_style_clusters() {
    let source = "\
#!/bin/bash
ps aux -p 1 -o comm=
ps ax -q 1 -o comm=
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let ps_commands = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("ps"))
        .collect::<Vec<_>>();

    assert_eq!(ps_commands.len(), 2);
    assert!(
        ps_commands
            .iter()
            .all(|fact| fact.options().ps().is_some_and(|ps| ps.has_pid_selector))
    );
}

#[test]
fn collects_base_prefix_arithmetic_spans_across_arithmetic_nodes() {
    let source = "\
#!/bin/bash
echo $((10#123))
echo ${foo:10#1:2}
: > \"$((10#1))\"
echo ${foo:-$((10#1))}
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert_eq!(
        facts
            .base_prefix_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["10#123", "10#1", "10#1", "10#1"]
    );
}

#[test]
fn ignores_base_prefix_like_parameter_trim_operands() {
    let source = "\
#!/bin/bash
: \"${progname:=\"${0##*/}\"}\"
echo ${foo:-${1##*/}}
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert!(facts.base_prefix_arithmetic_spans().is_empty());
}

#[test]
fn ignores_positional_parameter_trim_in_arithmetic_shell_words() {
    let source = "\
#!/bin/sh
echo $((42949 - ${1#-} / 100000))
";
    let output = Parser::with_dialect(source, shuck_parser::parser::ShellDialect::Posix)
        .parse()
        .unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert!(facts.base_prefix_arithmetic_spans().is_empty());
}

#[test]
fn collects_base_prefixes_in_arithmetic_parameter_defaults() {
    let source = "\
#!/bin/sh
echo $(( ${foo:-10#1} ))
";
    let output = Parser::with_dialect(source, shuck_parser::parser::ShellDialect::Posix)
        .parse()
        .unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert_eq!(
        facts
            .base_prefix_arithmetic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["10#1"]
    );
}

#[test]
fn builds_find_exec_shell_command_facts_for_execdir_shell_targets() {
    let source = "\
#!/bin/sh
# shellcheck disable=2086,2154
find $dir -type f -name \"rename*\" -execdir sh -c 'mv {} $(echo {} | sed \"s|rename|perl-rename|\")' \\;
";

    with_facts(source, None, |_, facts| {
        let find = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
            .expect("expected find -execdir fact");

        assert_eq!(find.effective_name(), Some("sh"));
        assert_eq!(find.wrappers(), &[WrapperKind::FindExecDir]);

        let find_exec_shell = find
            .options()
            .find_exec_shell()
            .expect("expected shell command fact for find -execdir");
        assert_eq!(
            find_exec_shell
                .shell_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'mv {} $(echo {} | sed \"s|rename|perl-rename|\")'"]
        );
    });
}

#[test]
fn builds_find_exec_shell_command_facts_for_exec_shell_targets() {
    let source = "\
#!/bin/sh
find . -exec bash -c 'hash=($(sha1sum {})); mv {} fuzz/corpus/$hash' \\;
";

    with_facts(source, None, |_, facts| {
        let find = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec) && fact.effective_name_is("bash"))
            .expect("expected find -exec fact");

        let find_exec_shell = find
            .options()
            .find_exec_shell()
            .expect("expected shell command fact for find -exec");
        assert_eq!(
            find_exec_shell
                .shell_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'hash=($(sha1sum {})); mv {} fuzz/corpus/$hash'"]
        );
    });
}

#[test]
fn builds_find_exec_shell_command_facts_for_later_exec_shell_targets() {
    let source = "\
#!/bin/sh
find . -exec echo {} + -name '*.cfg' -exec sh -c 'printf \"%s\\n\" {}' \\;
";

    with_facts(source, None, |_, facts| {
        let find = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec) && fact.effective_name_is("echo"))
            .expect("expected first find -exec fact");

        let find_exec_shell = find
            .options()
            .find_exec_shell()
            .expect("expected shell command fact for later find -exec");
        assert_eq!(
            find_exec_shell
                .shell_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'printf \"%s\\n\" {}'"]
        );
    });
}

#[test]
fn builds_find_exec_shell_command_facts_for_wrapped_shell_targets() {
    let source = "\
#!/bin/sh
find . -exec busybox sh -c 'printf \"%s\\n\" {}' \\;
find . -exec sudo sh -c 'printf \"%s\\n\" {}' \\;
";

    with_facts(source, None, |_, facts| {
        let shell_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.has_wrapper(WrapperKind::FindExec))
            .filter_map(|fact| fact.options().find_exec_shell())
            .flat_map(|find_exec_shell| find_exec_shell.shell_command_spans().iter())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            shell_spans,
            vec!["'printf \"%s\\n\" {}'", "'printf \"%s\\n\" {}'"]
        );
    });
}

#[test]
fn ignores_find_ok_shell_targets_for_find_exec_shell_command_facts() {
    let source = "\
#!/bin/sh
find . -ok sh -c 'printf \"%s\\n\" {}' \\;
find . -okdir bash -c 'printf \"%s\\n\" {}' \\;
";

    with_facts(source, None, |_, facts| {
        let shell_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().find_exec_shell())
            .flat_map(|find_exec_shell| find_exec_shell.shell_command_spans().iter())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(
            shell_spans.is_empty(),
            "unexpected shell spans: {shell_spans:?}"
        );
    });
}

#[test]
fn ignores_nested_find_exec_wrappers_for_find_exec_shell_command_facts() {
    let source = "\
#!/bin/sh
find . -exec find {} -ok sh -c 'printf \"%s\\n\" {}' \\; \\;
find . -execdir busybox find {} -okdir bash -c 'printf \"%s\\n\" {}' \\; \\;
";

    with_facts(source, None, |_, facts| {
        let shell_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().find_exec_shell())
            .flat_map(|find_exec_shell| find_exec_shell.shell_command_spans().iter())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(
            shell_spans.is_empty(),
            "unexpected shell spans: {shell_spans:?}"
        );
    });
}

#[test]
fn ignores_exec_tokens_nested_inside_find_ok_segments() {
    let source = "\
#!/bin/sh
find . -ok find {} -exec sh -c 'printf \"%s\\n\" {}' \\; \\;
find . -okdir busybox find {} -execdir bash -c 'printf \"%s\\n\" {}' \\; \\;
";

    with_facts(source, None, |_, facts| {
        let shell_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().find_exec_shell())
            .flat_map(|find_exec_shell| find_exec_shell.shell_command_spans().iter())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert!(
            shell_spans.is_empty(),
            "unexpected shell spans: {shell_spans:?}"
        );
    });
}

#[test]
fn builds_find_exec_argument_word_spans_for_wrapped_commands() {
    let source = "\
#!/bin/sh
find \"$root\"/*.py -exec echo \"$prefix\"*.tmp {} \\; -name '*.cfg'
result=$(find . -type d -name fuzz -exec dirname $(readlink -f {}) \\;)
find . -execdir sh -c 'printf \"%s\\n\" {}' {} \\;
";

    with_facts(source, None, |_, facts| {
        let top_level_find_exec = facts
            .commands()
            .iter()
            .find(|fact| {
                fact.has_wrapper(WrapperKind::FindExec)
                    && !fact.is_nested_word_command()
                    && fact.effective_name_is("echo")
            })
            .expect("expected top-level find -exec fact");
        assert_eq!(
            top_level_find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "\"$prefix\"*.tmp", "{}"]
        );

        let nested_find_exec = facts
            .commands()
            .iter()
            .find(|fact| {
                fact.has_wrapper(WrapperKind::FindExec)
                    && fact.is_nested_word_command()
                    && fact.effective_name_is("dirname")
            })
            .expect("expected nested find -exec fact");
        assert_eq!(
            nested_find_exec
                .options()
                .find_exec()
                .expect("expected nested find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["dirname", "$(readlink -f {})"]
        );

        let plus_argument_find_exec = facts
            .commands()
            .iter()
            .find(|fact| {
                fact.has_wrapper(WrapperKind::FindExec)
                    && !fact.is_nested_word_command()
                    && fact.effective_name_is("echo")
                    && fact.options().find_exec().is_some_and(|find_exec| {
                        find_exec
                            .argument_word_spans()
                            .iter()
                            .any(|span| span.slice(source) == "\"$prefix\"*.tmp")
                    })
            })
            .expect("expected semicolon-terminated find -exec fact");
        assert_eq!(
            plus_argument_find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "\"$prefix\"*.tmp", "{}"]
        );

        let find_execdir = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
            .expect("expected find -execdir fact");
        assert_eq!(
            find_execdir
                .options()
                .find_exec()
                .expect("expected find -exec facts for execdir wrapper")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["sh", "-c", "'printf \"%s\\n\" {}'", "{}"]
        );
    });
}

#[test]
fn keeps_plus_arguments_before_semicolon_terminated_find_exec() {
    let source = "#!/bin/sh\nfind . -exec echo + *.tmp {} \\;\n";

    with_facts(source, None, |_, facts| {
        let find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec))
            .expect("expected find -exec fact");

        assert_eq!(
            find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "+", "*.tmp", "{}"]
        );
    });
}

#[test]
fn keeps_quoted_backslash_semicolon_arguments_before_find_exec_terminator() {
    let source = "#!/bin/sh\nfind . -exec echo '\\;' *.tmp {} \\;\n";

    with_facts(source, None, |_, facts| {
        let find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec))
            .expect("expected find -exec fact");

        assert_eq!(
            find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "'\\;'", "*.tmp", "{}"]
        );
    });
}

#[test]
fn treats_quoted_semicolon_as_find_exec_terminator() {
    let source = "#!/bin/sh\nfind . -exec echo {} ';' -name *.cfg\n";

    with_facts(source, None, |_, facts| {
        let find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec))
            .expect("expected find -exec fact");

        assert_eq!(
            find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "{}"]
        );
    });
}

#[test]
fn stops_at_plus_terminator_before_later_exec_semicolon() {
    let source = "#!/bin/sh\nfind . -exec echo {} + -name *.cfg -exec rm {} \\;\n";

    with_facts(source, None, |_, facts| {
        let first_find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec) && fact.effective_name_is("echo"))
            .expect("expected first find -exec fact");

        assert_eq!(
            first_find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "{}", "rm", "{}"]
        );
    });
}

#[test]
fn stops_at_first_plus_terminator_before_later_exec_plus() {
    let source = "#!/bin/sh\nfind . -exec echo {} + -name *.cfg -exec rm {} +\n";

    with_facts(source, None, |_, facts| {
        let first_find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec) && fact.effective_name_is("echo"))
            .expect("expected first find -exec fact");

        assert_eq!(
            first_find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "{}", "rm", "{}"]
        );
    });
}

#[test]
fn builds_find_exec_argument_word_spans_for_dynamic_command_names() {
    let source = "#!/bin/sh\nfind . -exec \"$tool\" *.tmp {} \\;\n";

    with_facts(source, None, |_, facts| {
        let find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec))
            .expect("expected find -exec fact");

        assert_eq!(find_exec.effective_name(), None);
        assert_eq!(
            find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$tool\"", "*.tmp", "{}"]
        );
    });
}

#[test]
fn builds_find_exec_argument_word_spans_for_later_exec_clauses() {
    let source = "#!/bin/sh\nfind . -exec echo {} + -exec rm *.tmp {} +\n";

    with_facts(source, None, |_, facts| {
        let find_exec = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExec) && fact.effective_name_is("echo"))
            .expect("expected find -exec fact");

        assert_eq!(
            find_exec
                .options()
                .find_exec()
                .expect("expected find -exec facts")
                .argument_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo", "{}", "rm", "*.tmp", "{}"]
        );
    });
}

#[test]
fn builds_find_exec_shell_command_facts_for_bundled_execdir_shell_c_flags() {
    let source = "\
#!/bin/sh
find . -execdir sh -ec 'mv {} \"$target\"' \\;
";

    with_facts(source, None, |_, facts| {
        let find = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
            .expect("expected find -execdir fact");

        let find_exec_shell = find
            .options()
            .find_exec_shell()
            .expect("expected shell command fact for bundled -c flags");
        assert_eq!(
            find_exec_shell
                .shell_command_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'mv {} \"$target\"'"]
        );
    });
}

#[test]
fn summarizes_builtin_wrapped_reads() {
    let source = "#!/bin/bash\nbuiltin read response\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let read = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("read"))
        .expect("expected builtin-wrapped read fact");

    assert_eq!(read.wrappers(), &[WrapperKind::Builtin]);
    assert_eq!(
        read.options().read().map(|read| read.uses_raw_input),
        Some(false)
    );
}

#[test]
fn summarizes_read_array_targets_without_trailing_names() {
    let source = "\
#!/bin/bash
read first second
read -a arr name
read -aarr name
read -ar name
read -- -a name
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let reads = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("read"))
        .collect::<Vec<_>>();

    assert_eq!(
        reads[0]
            .options()
            .read()
            .expect("expected plain read facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert_eq!(
        reads[1]
            .options()
            .read()
            .expect("expected read -a facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["arr"]
    );
    assert_eq!(
        reads[2]
            .options()
            .read()
            .expect("expected read -aNAME facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["arr"]
    );
    assert_eq!(
        reads[3]
            .options()
            .read()
            .expect("expected read -ar facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["r"]
    );
    assert_eq!(
        reads[4]
            .options()
            .read()
            .expect("expected read -- facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["name"]
    );
}

#[test]
fn summarizes_quoted_read_target_names() {
    let source = "\
#!/bin/bash
read \"path\" 'name'
read -a \"items\" trailing
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let reads = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("read"))
        .collect::<Vec<_>>();

    assert_eq!(
        reads[0]
            .options()
            .read()
            .expect("expected quoted read facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["\"path\"", "'name'"]
    );
    assert_eq!(
        reads[1]
            .options()
            .read()
            .expect("expected quoted read -a facts")
            .target_name_uses()
            .iter()
            .map(|name_use| name_use.span().slice(source))
            .collect::<Vec<_>>(),
        vec!["\"items\""]
    );
}

#[test]
fn summarizes_su_login_forms() {
    let source = "\
#!/bin/bash
su root
su root -c id
su \"$user\" -s /bin/sh -c \"$cmd\"
su -s /bin/sh root
su -
su --login root
su -l
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let su_login_flags = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("su"))
        .map(|fact| fact.options().su().map(|su| su.has_login_flag()))
        .collect::<Vec<_>>();

    assert_eq!(
        su_login_flags,
        vec![
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(true),
            Some(true),
            Some(true)
        ]
    );
}

#[test]
fn keeps_only_login_aliases_marked_as_login_forms() {
    let source = "\
#!/bin/bash
su -c
su --command
su -m root
su -- root
su root -s /bin/sh
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let su_login_flags = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("su"))
        .map(|fact| fact.options().su().map(|su| su.has_login_flag()))
        .collect::<Vec<_>>();

    assert_eq!(
        su_login_flags,
        vec![
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false)
        ]
    );
}

#[test]
fn summarizes_first_nonportable_sh_builtin_option_words() {
    let source = "\
#!/bin/sh
read -r name
read -p prompt name
read -\"$mode\" name
printf -v out '%s' foo
printf -- -v out
export -p
export -fn foo
command export -fn foo
trap -p EXIT
trap -- -p EXIT
wait -n
wait -p jobid -n
ulimit -f
ulimit -n
type -P printf
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Bash,
        ShellDialect::Sh,
        |_, facts| {
            let spans = facts
                .commands()
                .iter()
                .filter_map(|fact| {
                    fact.options()
                        .nonportable_sh_builtin_option_span()
                        .map(|span| span.slice(source))
                })
                .collect::<Vec<_>>();

            assert_eq!(
                spans,
                vec![
                    "-p",
                    "-\"$mode\"",
                    "-v",
                    "-fn",
                    "-fn",
                    "-p",
                    "-n",
                    "-p",
                    "-n",
                    "-P"
                ]
            );
        },
    );
}

#[test]
fn set_command_flags_without_prefix_ignore_quoted_literals() {
    let source = "\
set foo bar
set \"foo\" bar
set f\"oo\" bar
set 'foo' bar
";

    with_facts(source, None, |_, facts| {
        let set_without_prefix_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("set"))
            .filter_map(|fact| fact.options().set())
            .flat_map(|set| set.flags_without_prefix_spans().iter().copied())
            .collect::<Vec<_>>();

        assert_eq!(
            set_without_prefix_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo"]
        );
    });
}

#[test]
fn summarizes_directory_change_commands_and_errexit_hints() {
    let source = "\n#!/bin/bash -eu\ncd ../..\ncd -\nbuiltin cd /\npushd ..\npopd\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert!(facts.errexit_enabled_anywhere());

    let directory_changes = facts
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.options().directory_change().map(|directory_change| {
                (
                    directory_change.command_name(),
                    directory_change.is_plain_directory_stack_marker(),
                    directory_change.is_manual_restore_candidate(),
                    fact.wrappers().to_vec(),
                )
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(
        directory_changes,
        vec![
            ("cd", true, false, vec![]),
            ("cd", false, true, vec![]),
            ("cd", false, false, vec![WrapperKind::Builtin]),
            ("pushd", true, false, vec![]),
            ("popd", false, false, vec![])
        ]
    );
}

#[test]
fn does_not_treat_long_shebang_options_as_errexit() {
    let source = "#!/bin/bash --noprofile\ncd /tmp\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    assert!(!facts.errexit_enabled_anywhere());
}

#[test]
fn keeps_read_raw_input_when_option_flags_are_dynamic() {
    let source = "#!/bin/bash\nbuiltin read -${_read_char_flag} 1 -s -r anykey\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let read = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("read"))
        .expect("expected dynamic-option read fact");

    assert_eq!(read.wrappers(), &[WrapperKind::Builtin]);
    assert_eq!(
        read.options().read().map(|read| read.uses_raw_input),
        Some(true)
    );
}

#[test]
fn resolves_sudo_family_invokers_through_outer_wrappers() {
    let source = "#!/bin/bash\ncommand sudo tee out.txt\ncommand run0 tee out.txt\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let invokers = facts
        .commands()
        .iter()
        .filter_map(|fact| fact.options().sudo_family().map(|sudo| sudo.invoker))
        .collect::<Vec<_>>();

    assert_eq!(
        invokers,
        vec![SudoFamilyInvoker::Sudo, SudoFamilyInvoker::Run0]
    );
}

#[test]
fn resolves_sudo_family_invokers_when_wrapper_names_are_escaped() {
    let source = "#!/bin/bash\n\\command \\doas tee out.txt\n\\command \\sudo tee out.txt\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let invokers = facts
        .commands()
        .iter()
        .filter_map(|fact| fact.options().sudo_family().map(|sudo| sudo.invoker))
        .collect::<Vec<_>>();

    assert_eq!(
        invokers,
        vec![SudoFamilyInvoker::Doas, SudoFamilyInvoker::Sudo]
    );
}

#[test]
fn resolves_sudo_family_invokers_when_wrapper_target_is_unresolved() {
    let source = "\
#!/bin/bash
sudo \"$tool\" > out.txt
sudo -V
command run0 --version
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let invokers = facts
        .commands()
        .iter()
        .filter_map(|fact| fact.options().sudo_family().map(|sudo| sudo.invoker))
        .collect::<Vec<_>>();

    assert_eq!(
        invokers,
        vec![
            SudoFamilyInvoker::Sudo,
            SudoFamilyInvoker::Sudo,
            SudoFamilyInvoker::Run0,
        ]
    );
}

#[test]
fn parses_long_xargs_null_mode_and_numeric_exit_status() {
    let source = "#!/bin/bash\nfind . -print0 | xargs --null rm\nexit 42\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let xargs = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("xargs"))
        .and_then(|fact| fact.options().xargs())
        .expect("expected xargs facts");
    assert!(xargs.uses_null_input);
    assert!(xargs.inline_replace_option_spans().is_empty());

    let exit = facts
        .commands()
        .iter()
        .find(|fact| fact.options().exit().is_some())
        .and_then(|fact| fact.options().exit())
        .expect("expected exit facts");
    assert_eq!(
        exit.status_word.map(|word| word.span.slice(source)),
        Some("42")
    );
    assert!(exit.has_static_status());
    assert!(exit.is_numeric_literal);
    assert!(!exit.has_invalid_status_argument());
}

#[test]
fn parses_mixed_and_pure_dynamic_exit_status_shapes() {
    let source = "\
#!/bin/bash
code=3
other=4
exit \"message $code\"
exit \"123$code\"
exit \"$code\"
exit \"${code}${other}\"
exit \"$(printf '%s' 3)\"
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let exits = facts
        .commands()
        .iter()
        .filter_map(|fact| fact.options().exit())
        .collect::<Vec<_>>();

    assert_eq!(
        exits
            .iter()
            .map(|exit| exit.status_word.map(|word| word.span.slice(source)))
            .collect::<Vec<_>>(),
        vec![
            Some("\"message $code\""),
            Some("\"123$code\""),
            Some("\"$code\""),
            Some("\"${code}${other}\""),
            Some("\"$(printf '%s' 3)\""),
        ]
    );
    assert_eq!(
        exits
            .iter()
            .map(|exit| exit.has_invalid_status_argument())
            .collect::<Vec<_>>(),
        vec![true, true, false, false, false]
    );
}

#[test]
fn keeps_parsing_xargs_flags_after_optional_argument_forms() {
    let source = "\
#!/bin/bash
find . -print0 | xargs -l -0 rm
find . -print0 | xargs --eof --null rm
xargs -i0 echo
xargs -i bash -c 'echo {}'
xargs -0i echo '-----> Configuring {}'
xargs -i echo \"-----> Configuring {} with $template\"
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let xargs_facts = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("xargs"))
        .filter_map(|fact| fact.options().xargs())
        .collect::<Vec<_>>();

    assert_eq!(xargs_facts.len(), 6);
    assert!(xargs_facts[0].uses_null_input);
    assert!(xargs_facts[1].uses_null_input);
    assert!(!xargs_facts[2].uses_null_input);
    assert_eq!(
        xargs_facts[2]
            .inline_replace_option_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["-i0"]
    );
    assert!(xargs_facts[3].inline_replace_option_spans().is_empty());
    assert!(xargs_facts[4].inline_replace_option_spans().is_empty());
    assert!(xargs_facts[5].inline_replace_option_spans().is_empty());
}

#[test]
fn does_not_consume_null_mode_after_optional_long_eof() {
    let source = "#!/bin/bash\nfind . -print0 | xargs --eof --null rm\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let xargs = facts
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("xargs"))
        .and_then(|fact| fact.options().xargs())
        .expect("expected xargs facts");

    assert!(xargs.uses_null_input);
}

#[test]
fn keeps_parsing_xargs_flags_after_arg_file() {
    let source = "\
#!/bin/bash
find . -print0 | xargs -a inputs -0 rm
xargs -a inputs -iX echo X
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let xargs_facts = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("xargs"))
        .filter_map(|fact| fact.options().xargs())
        .collect::<Vec<_>>();

    assert_eq!(xargs_facts.len(), 2);
    assert!(xargs_facts[0].uses_null_input);
    assert_eq!(
        xargs_facts[1]
            .inline_replace_option_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>(),
        vec!["-iX"]
    );
    assert_eq!(xargs_facts[0].max_procs(), None);
    assert_eq!(xargs_facts[1].max_procs(), None);
}

#[test]
fn parses_xargs_max_procs_values() {
    let source = "\
#!/bin/bash
find . | xargs -P10 echo
find . | xargs -P1 echo
find . | xargs -P 0 echo
find . | xargs --max-procs=10 echo
find . | xargs --max-procs \"$N\" echo
find . | xargs -P11 echo
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, None, ShellDialect::Bash);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

    let xargs_facts = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("xargs"))
        .filter_map(|fact| fact.options().xargs())
        .collect::<Vec<_>>();

    assert_eq!(xargs_facts.len(), 6);
    assert_eq!(
        xargs_facts
            .iter()
            .map(|xargs| xargs.max_procs())
            .collect::<Vec<_>>(),
        vec![Some(10), Some(1), Some(0), Some(10), None, Some(11)]
    );
    assert_eq!(
        xargs_facts
            .iter()
            .map(|xargs| xargs.has_zero_digit_option_word())
            .collect::<Vec<_>>(),
        vec![true, false, false, true, false, false]
    );
}

#[test]
fn rm_command_facts_track_shellcheck_style_variable_path_hazards() {
    let source = "\
#!/bin/bash
PKG=/pkg
PRGNAM=demo
DESTDIR=/dest
PYDIR=/py
SUFFIX=
LIBDIRSUFFIX=64
rm -rf $PKG/usr
rm -rf $PKG/usr/share/$PRGNAM
rm -rf \"$DESTDIR\"/usr
rm -rf $PKG/usr/{bin,include,libexec,man,share}
rm -rf \"$PKG/$PYDIR/usr\"
rm -rf $PKG/$PYDIR/*
rm -rf \"$DESTDIR\"/${PRGNAM}*
rm -rf \"$DESTDIR\"/usr${SUFFIX}
rm -rf \"$DESTDIR\"/usr${SUFFIX}/$PRGNAM
rm -rf \"$DESTDIR\"/usr/${PRGNAM}*
rm -rf \"$DESTDIR\"/lib/${PRGNAM}*
rm -rf $PKG/$PYDIR/lib*
rm -rf \"$DESTDIR\"/lib*
rm -rf \"$DESTDIR\"/opt
rm -rf \"$DESTDIR\"/opt/$PRGNAM
rm -rf $PKG/usr/share/doc
rm -rf $PKG/usr/share/icons
rm -rf $PKG/usr/doc/$PRGNAM
rm -rf $PKG/usr/lib${LIBDIRSUFFIX}/*.la
rm -rf $PKG/usr/share/$PRGNAM/icons
rm -rf $PKG/opt/$PRGNAM/bin
";

    with_facts(source, None, |_, facts| {
        let rm_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().rm())
            .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            rm_spans,
            vec![
                "$PKG/usr",
                "$PKG/usr/share/$PRGNAM",
                "\"$DESTDIR\"/usr",
                "$PKG/usr/{bin,include,libexec,man,share}",
                "$PKG/usr/{bin,include,libexec,man,share}",
                "\"$PKG/$PYDIR/usr\"",
                "$PKG/$PYDIR/*",
                "\"$DESTDIR\"/${PRGNAM}*",
                "\"$DESTDIR\"/usr${SUFFIX}",
                "\"$DESTDIR\"/usr${SUFFIX}/$PRGNAM",
                "\"$DESTDIR\"/usr/${PRGNAM}*",
                "\"$DESTDIR\"/lib/${PRGNAM}*",
            ]
        );
    });
}

#[test]
fn rm_command_facts_match_shellcheck_only_k001_shapes() {
    let source = "\
#!/bin/bash
PKG=/pkg
PRGNAM=demo
ITEM='*.exe'
DESTDIR=/dest
SYSROOT=/target
PACKAGE=/archive
rm -rf /usr/share/$PRGNAM
rm -rf $PKG/usr/share/$PRGNAM/$ITEM
rm -rf $PACKAGE/
rm -rf $PKG/usr/{bin,include,share}
rm -rf ${DESTDIR}/${SYSROOT}/{sbin,etc,var,libexec}
";

    with_facts(source, None, |_, facts| {
        let rm_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().rm())
            .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            rm_spans,
            vec![
                "/usr/share/$PRGNAM",
                "$PKG/usr/share/$PRGNAM/$ITEM",
                "$PACKAGE/",
                "$PKG/usr/{bin,include,share}",
                "$PKG/usr/{bin,include,share}",
                "${DESTDIR}/${SYSROOT}/{sbin,etc,var,libexec}",
                "${DESTDIR}/${SYSROOT}/{sbin,etc,var,libexec}",
            ]
        );
    });
}

#[test]
fn rm_command_facts_flag_literal_system_prefix_globs() {
    let source = "\
#!/bin/bash
rm -rf /usr/*
rm -rf /usr/share/*
";

    with_facts(source, None, |_, facts| {
        let rm_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().rm())
            .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(rm_spans, vec!["/usr/*", "/usr/share/*"]);
    });
}

#[test]
fn builds_redirect_facts_with_cached_target_analysis() {
    let source = "#!/bin/bash\necho hi 2>&3 >/dev/null >> \"$((i++))\"\necho hi > \"$((i + 1))\"\n";

    with_facts(source, None, |_, facts| {
        let command = facts
            .structural_commands()
            .find(|fact| fact.effective_name_is("echo"))
            .expect("expected echo fact");

        let redirects = command.redirect_facts();
        assert_eq!(redirects.len(), 3);

        let descriptor_dup = &redirects[0];
        assert_eq!(descriptor_dup.operator_span().slice(source), ">&");
        assert!(
            descriptor_dup
                .analysis()
                .is_some_and(|analysis| analysis.is_descriptor_dup())
        );
        assert_eq!(
            descriptor_dup
                .analysis()
                .and_then(|analysis| analysis.numeric_descriptor_target),
            Some(3)
        );

        let dev_null = &redirects[1];
        assert_eq!(dev_null.operator_span().slice(source), ">");
        assert_eq!(
            dev_null.target_span().map(|span| span.slice(source)),
            Some("/dev/null")
        );
        assert!(
            dev_null
                .analysis()
                .is_some_and(|analysis| analysis.is_definitely_dev_null())
        );

        let arithmetic = &redirects[2];
        assert_eq!(arithmetic.operator_span().slice(source), ">>");
        assert_eq!(
            arithmetic.target_span().map(|span| span.slice(source)),
            Some("\"$((i++))\"")
        );
        assert!(
            arithmetic
                .analysis()
                .is_some_and(|analysis| { analysis.expansion.hazards.arithmetic_expansion })
        );
        assert_eq!(
            arithmetic
                .arithmetic_update_operator_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++"]
        );

        let pure_arithmetic = facts
            .structural_commands()
            .filter(|fact| fact.effective_name_is("echo"))
            .nth(1)
            .expect("expected second echo fact");
        let pure_redirect = &pure_arithmetic.redirect_facts()[0];
        assert!(
            pure_redirect
                .analysis()
                .is_some_and(|analysis| { analysis.expansion.hazards.arithmetic_expansion })
        );
        assert!(pure_redirect.arithmetic_update_operator_spans().is_empty());
    });
}

#[test]
fn builds_substitution_facts_with_intent_and_host_kinds() {
    let source = "\
#!/bin/bash
printf '%s\\n' $(printf arg) \"$(printf quoted)\"
local decl_assign=$(printf decl-assign)
name[$(printf assign)]=1
declare arr[$(printf decl-name)]
declare other=$(printf decl-assign-2)
declare -A map=([$(printf key)]=1)
bucket[$(ls | wc -l)]=1
branch[$(if true; then ls | wc -l; fi)]=1
nested[$(echo \"$(ls | wc -l)\")]=1
cat <<<$(printf here)
out=$(printf hi > out.txt)
drop=$(printf hi >/dev/null 2>&1)
quiet=$(printf hi &>/dev/null)
mixed=$(jq -r . <<< \"$status\" || die >&2)
x=$(echo direct)
y=$(foo $(echo nested))
z=$(ls layout.*.h | cut -d. -f2 | xargs echo)
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.stdout_intent(),
                    fact.host_kind(),
                    fact.unquoted_in_host(),
                    fact.body_contains_ls(),
                    fact.body_contains_echo(),
                )
            })
            .collect::<Vec<_>>();

        assert!(substitutions.contains(&(
            "$(printf arg)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::CommandArgument,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf decl-assign)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::DeclarationAssignmentValue,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf quoted)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::CommandArgument,
            false,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf here)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::HereStringOperand,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf assign)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::AssignmentTargetSubscript,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf decl-name)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::DeclarationNameSubscript,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf key)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::ArrayKeySubscript,
            true,
            false,
            false,
        )));
        assert_eq!(
            facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts())
                .find(|fact| fact.span().slice(source) == "$(ls | wc -l)")
                .expect("expected assignment subscript ls pipeline substitution")
                .body_processed_ls_pipeline_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ls"]
        );
        assert_eq!(
            facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts())
                .find(|fact| fact.span().slice(source) == "$(if true; then ls | wc -l; fi)")
                .expect("expected assignment subscript branch ls pipeline substitution")
                .body_processed_ls_pipeline_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ls"]
        );
        assert_eq!(
            facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts())
                .find(|fact| fact.span().slice(source) == "$(echo \"$(ls | wc -l)\")")
                .expect("expected assignment subscript nested ls pipeline substitution")
                .body_processed_ls_pipeline_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ls"]
        );
        assert!(substitutions.contains(&(
            "$(printf hi > out.txt)".to_owned(),
            SubstitutionOutputIntent::Rerouted,
            SubstitutionHostKind::Other,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf hi >/dev/null 2>&1)".to_owned(),
            SubstitutionOutputIntent::Discarded,
            SubstitutionHostKind::Other,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(printf hi &>/dev/null)".to_owned(),
            SubstitutionOutputIntent::Discarded,
            SubstitutionHostKind::Other,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(jq -r . <<< \"$status\" || die >&2)".to_owned(),
            SubstitutionOutputIntent::Mixed,
            SubstitutionHostKind::Other,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(echo direct)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::Other,
            true,
            false,
            true,
        )));
        assert!(substitutions.contains(&(
            "$(foo $(echo nested))".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::Other,
            true,
            false,
            false,
        )));
        assert!(substitutions.contains(&(
            "$(echo nested)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::CommandArgument,
            true,
            false,
            true,
        )));
        assert!(substitutions.contains(&(
            "$(ls layout.*.h | cut -d. -f2 | xargs echo)".to_owned(),
            SubstitutionOutputIntent::Captured,
            SubstitutionHostKind::Other,
            true,
            true,
            false,
        )));
    });
}

#[test]
fn excludes_output_both_dev_null_redirects_from_c058_fix_spans() {
    let source = "\
#!/bin/bash
drop=$(printf hi >/dev/null 2>&1)
quiet=$(printf hi &>/dev/null)
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .collect::<Vec<_>>();

        let drop = substitutions
            .iter()
            .copied()
            .find(|fact| fact.span().slice(source) == "$(printf hi >/dev/null 2>&1)")
            .expect("expected explicit redirect substitution fact");
        assert_eq!(
            drop.stdout_dev_null_redirect_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/null"]
        );

        let quiet = substitutions
            .iter()
            .copied()
            .find(|fact| fact.span().slice(source) == "$(printf hi &>/dev/null)")
            .expect("expected output-both substitution fact");
        assert!(quiet.stdout_redirect_spans().is_empty());
        assert!(quiet.stdout_dev_null_redirect_spans().is_empty());
    });
}

#[test]
fn uses_pipeline_tail_redirects_for_substitution_output_intent() {
    let source = "\
#!/bin/sh
out=$(printf '%s\\n' \"$pkg\" | grep '^ok$' >/dev/null 2>&1)
";

    with_facts(source, None, |_, facts| {
        let substitution = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .find(|fact| {
                fact.span().slice(source)
                    == "$(printf '%s\\n' \"$pkg\" | grep '^ok$' >/dev/null 2>&1)"
            })
            .expect("expected pipeline substitution fact");

        assert_eq!(
            substitution.stdout_intent(),
            SubstitutionOutputIntent::Discarded
        );
        assert_eq!(
            substitution
                .stdout_dev_null_redirect_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/null"]
        );
    });
}

#[test]
fn outer_stderr_capture_keeps_grouped_substitution_output_captured() {
    let source = "\
#!/bin/sh
error=$({ printf '%s\\n' boom >/dev/null; } 2>&1)
";

    with_facts(source, None, |_, facts| {
        let substitution = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .find(|fact| fact.span().slice(source) == "$({ printf '%s\\n' boom >/dev/null; } 2>&1)")
            .expect("expected grouped substitution fact");

        assert_eq!(
            substitution.stdout_intent(),
            SubstitutionOutputIntent::Captured
        );
        assert!(substitution.stdout_redirect_spans().is_empty());
        assert!(substitution.stdout_dev_null_redirect_spans().is_empty());
    });
}

#[test]
fn treats_stderr_capture_before_stdout_redirect_as_captured_substitution_output() {
    let source = "#!/bin/sh\nchoice=$(printf hi 2>&1 >/dev/tty)\n";

    with_facts(source, None, |_, facts| {
        let substitution = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .find(|fact| fact.span().slice(source) == "$(printf hi 2>&1 >/dev/tty)")
            .expect("expected substitution fact");

        assert_eq!(
            substitution.stdout_intent(),
            SubstitutionOutputIntent::Captured
        );
        assert_eq!(
            substitution
                .stdout_redirect_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/tty"]
        );
    });
}

#[test]
fn applies_outer_compound_redirects_to_substitution_output() {
    let source = "\
#!/bin/sh
quiet=$({ printf hi; } >/dev/null)
shown=$({ printf hi; } >/dev/tty)
";

    with_facts(source, None, |_, facts| {
        let quiet = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .find(|fact| fact.span().slice(source) == "$({ printf hi; } >/dev/null)")
            .expect("expected grouped substitution fact");
        let shown = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .find(|fact| fact.span().slice(source) == "$({ printf hi; } >/dev/tty)")
            .expect("expected grouped substitution fact");

        assert_eq!(quiet.stdout_intent(), SubstitutionOutputIntent::Discarded);
        assert_eq!(
            quiet
                .stdout_redirect_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/null"]
        );
        assert_eq!(
            quiet
                .stdout_dev_null_redirect_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/null"]
        );

        assert_eq!(shown.stdout_intent(), SubstitutionOutputIntent::Rerouted);
        assert_eq!(
            shown
                .stdout_redirect_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/tty"]
        );
        assert!(shown.stdout_dev_null_redirect_spans().is_empty());
    });
}

#[test]
fn leaves_inner_compound_redirects_out_of_substitution_output_warnings() {
    let source = "\
#!/bin/sh
file=$({ printf hi >out.txt; })
fd=$({ printf hi >&5; })
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .collect::<Vec<_>>();

        for substitution in substitutions {
            assert_eq!(
                substitution.stdout_intent(),
                SubstitutionOutputIntent::Captured
            );
            assert!(substitution.stdout_redirect_spans().is_empty());
            assert!(substitution.stdout_dev_null_redirect_spans().is_empty());
        }
    });
}

#[test]
fn builds_docker_ps_substitution_facts_without_pgrep_exemptions() {
    let source = "\
#!/bin/bash
docker inspect -f '{{ if ne \"true\" (index .Config.Labels \"com.dokku.devcontainer\") }}{{.ID}} {{ end }}' $(docker ps -q)
";

    with_facts(source, None, |_, facts| {
        let substitution = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter())
            .find(|fact| fact.span().slice(source) == "$(docker ps -q)")
            .expect("expected docker ps substitution fact");

        assert_eq!(
            substitution.host_kind(),
            SubstitutionHostKind::CommandArgument
        );
        assert!(substitution.unquoted_in_host());
        assert!(substitution.body_has_commands());
        assert!(!substitution.body_is_pgrep_lookup());
        assert!(!substitution.body_is_seq_utility());
    });
}

#[test]
fn tracks_backtick_syntax_in_substitution_facts() {
    let source = "\
#!/bin/sh
printf '%s\\n' `date` $(uname) <(cat /etc/hosts)
";

    with_facts(source, None, |_, facts| {
        let substitutions = facts
            .commands()
            .iter()
            .flat_map(|fact| fact.substitution_facts().iter().cloned())
            .map(|fact| {
                (
                    fact.span().slice(source).to_owned(),
                    fact.command_syntax(),
                    fact.uses_backtick_syntax(),
                )
            })
            .collect::<Vec<_>>();

        assert!(substitutions.contains(&(
            "`date`".to_owned(),
            Some(CommandSubstitutionSyntax::Backtick),
            true,
        )));
        assert!(substitutions.contains(&(
            "$(uname)".to_owned(),
            Some(CommandSubstitutionSyntax::DollarParen),
            false,
        )));
        assert!(substitutions.contains(&("<(cat /etc/hosts)".to_owned(), None, false,)));
    });
}
