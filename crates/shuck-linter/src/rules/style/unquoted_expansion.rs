use rustc_hash::FxHashSet;

use crate::{
    Checker, ExpansionContext, Rule, SafeValueIndex, SafeValueQuery, ShellDialect, Violation,
    WordOccurrenceRef,
};

pub struct UnquotedExpansion;

impl Violation for UnquotedExpansion {
    fn rule() -> Rule {
        Rule::UnquotedExpansion
    }

    fn message(&self) -> String {
        "quote parameter expansions to avoid word splitting and globbing".to_owned()
    }
}

pub fn unquoted_expansion(checker: &mut Checker) {
    let source = checker.source();
    let colon_command_ids = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is(":"))
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();
    let mut safe_values = SafeValueIndex::build(
        checker.semantic(),
        checker.semantic_analysis(),
        checker.facts(),
        source,
    );

    let mut spans = Vec::new();
    for fact in checker.facts().word_facts() {
        let Some(context) = fact.expansion_context() else {
            continue;
        };
        if !should_check_context(context, checker.shell()) {
            continue;
        }

        report_word_expansions(
            &mut spans,
            &mut safe_values,
            fact,
            context,
            colon_command_ids.contains(&fact.command_id()),
        );
    }

    drop(safe_values);

    for span in spans {
        checker.report_dedup(UnquotedExpansion, span);
    }
}

fn should_check_context(context: ExpansionContext, shell: ShellDialect) -> bool {
    match context {
        ExpansionContext::CommandName
        | ExpansionContext::CommandArgument
        | ExpansionContext::HereString
        | ExpansionContext::RedirectTarget(_) => true,
        ExpansionContext::DeclarationAssignmentValue => shell != ShellDialect::Bash,
        _ => false,
    }
}

fn report_word_expansions(
    spans: &mut Vec<shuck_ast::Span>,
    safe_values: &mut SafeValueIndex<'_>,
    fact: WordOccurrenceRef<'_, '_>,
    context: ExpansionContext,
    in_colon_command: bool,
) {
    if !fact.analysis().hazards.field_splitting && !fact.analysis().hazards.pathname_matching {
        return;
    }

    let scalar_spans = fact.scalar_expansion_spans();
    let assign_default_spans = if in_colon_command && context == ExpansionContext::CommandArgument {
        fact.unquoted_assign_default_spans()
    } else {
        Default::default()
    };
    let use_replacement_spans = fact.use_replacement_spans();
    let star_spans = fact.unquoted_star_parameter_spans();
    if scalar_spans.is_empty() && star_spans.is_empty() {
        return;
    }
    if context == ExpansionContext::CommandName
        && !fact.has_literal_affixes()
        && fact.parts_len() == 1
    {
        return;
    }
    let Some(query) = SafeValueQuery::from_context(context) else {
        return;
    };

    for (part, part_span) in fact.parts_with_spans() {
        let report_unquoted_star = star_spans.contains(&part_span);
        if !scalar_spans.contains(&part_span) && !report_unquoted_star {
            continue;
        }
        if assign_default_spans.contains(&part_span) {
            continue;
        }
        if use_replacement_spans.contains(&part_span) {
            continue;
        }
        if safe_values.part_is_safe(part, part_span, query) {
            continue;
        }

        spans.push(part_span);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_scalar_expansion_parts_instead_of_whole_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${name}suffix ${arr[0]} ${arr[@]}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}", "${arr[0]}"]
        );
    }

    #[test]
    fn descends_into_nested_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$(echo $name)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn ignores_expansions_inside_quoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
exec dbus-send --bus=\"unix:path=$XDG_RUNTIME_DIR/bus\" / org.freedesktop.DBus.Peer.Ping
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_unquoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix\"$HOME\"/$suffix
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix"]
        );
    }

    #[test]
    fn skips_for_lists_but_reports_here_strings_and_redirect_targets() {
        let source = "\
#!/bin/bash
for item in $first \"$second\"; do :; done
cat <<< $here >$out
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$here", "$out"]
        );
    }

    #[test]
    fn skips_assignment_values_and_descriptor_dup_targets() {
        let source = "\
#!/bin/bash
value=$name
printf '%s\\n' ok >&$fd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unquoted_zsh_parameter_modifiers() {
        let source = "\
#!/usr/bin/env zsh
print ${~foo}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${~foo}"]
        );
    }

    #[test]
    fn reports_dynamic_command_names() {
        let source = "\
#!/bin/bash
$HOME/bin/tool $arg
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOME", "$arg"]
        );
    }

    #[test]
    fn reports_bourne_transformations_in_command_arguments() {
        let source = "\
#!/bin/bash
printf '%s\\n' ${name@U}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name@U}"]
        );
    }

    #[test]
    fn reports_bindings_derived_from_parameter_operations() {
        let source = "\
#!/bin/bash
PRGNAM=Fennel
SRCNAM=${PRGNAM,}
release=1.0.0
VERSION=${release:-fallback}
rm -rf $SRCNAM-$VERSION
printf '%s\\n' ${PRGNAM,} ${release:-fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SRCNAM", "$VERSION"]
        );
    }

    #[test]
    fn reports_bindings_from_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
check() { return 0; }
check && w='-w' || w=''
if check; then
  flag='-w'
else
  flag=''
fi
iptables $w -t nat -N chain
iptables $flag -t nat -N chain
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$w"]
        );
    }

    #[test]
    fn reports_nested_guarded_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
f() {
  [ \"$1\" = iptables ] && {
    true && w='-w' || w=''
  }
  [ \"$1\" = ip6tables ] && {
    true && w='-w' || w=''
  }
  iptables $w -t nat -N chain
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$w"]
        );
    }

    #[test]
    fn skips_colon_assign_default_expansions_but_keeps_regular_argument_cases() {
        let source = "\
#!/bin/bash
: ${x:=fallback} $other
printf '%s\\n' ${z:=fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.slice(source)))
                .collect::<Vec<_>>(),
            vec![(2, "$other"), (3, "${z:=fallback}")]
        );
    }

    #[test]
    fn keeps_colon_assign_default_reports_for_here_strings_and_redirect_targets() {
        let source = "\
#!/bin/bash
: <<< ${x:=fallback} >${y:=out}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${x:=fallback}", "${y:=out}"]
        );
    }

    #[test]
    fn skips_use_replacement_expansions() {
        let source = "\
#!/bin/bash
foo='a b'
arr=('left side' right)
printf '%s\\n' ${foo:+$foo} ${foo:+\"$foo\"} ${arr:+\"${arr[@]}\"}
tar ${foo:+-C \"$foo\"} -f archive.tar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn keeps_default_expansions_with_quoted_operands() {
        let source = "\
#!/bin/bash
foo='a b'
printf '%s\\n' ${foo:-\"$foo\"}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${foo:-\"$foo\"}"]
        );
    }

    #[test]
    fn skips_plain_expansion_command_names_but_reports_composite_command_words() {
        let source = "\
#!/bin/bash
$CC -c file.c
if $TERMUX_ON_DEVICE_BUILD; then
  :
fi
${CC}${FLAGS} file.c
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CC}", "${FLAGS}"]
        );
    }

    #[test]
    fn ignores_escaped_backticks_inside_double_quoted_assignments() {
        let source = "\
#!/bin/bash
NVM_TEST_VERSION=v0.42
EXPECTED=\"Found '$(pwd)/.nvmrc' with version <${NVM_TEST_VERSION}>
N/A: version \\\"${NVM_TEST_VERSION}\\\" is not yet installed.

You need to run \\`nvm install ${NVM_TEST_VERSION}\\` to install and use it.
No NODE_VERSION provided; no .nvmrc file found\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn reports_expansions_wrapped_in_escaped_literal_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' -DPACKAGE_VERSION=\\\"$TERMUX_PKG_VERSION\\\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_VERSION"]
        );
    }

    #[test]
    fn reports_decl_assignment_values_in_sh_mode() {
        let source = "\
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_BUILDER_DIR", "$HOME", "$PATH"]
        );
    }

    #[test]
    fn reports_transformed_decl_assignment_values_in_sh_mode() {
        let source = "\
local upper=${TERMUX_ARCH@U}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${TERMUX_ARCH@U}"]
        );
    }

    #[test]
    fn skips_decl_assignment_values_in_bash_mode() {
        let source = "\
#!/bin/bash
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.bash"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unquoted_spans_inside_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'prefix:'$name':suffix'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn skips_safe_special_parameters() {
        let source = "\
#!/bin/bash
printf '%s\\n' $? $# $$ $! $- $0 $1 $* $@
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$1", "$*"]
        );
    }

    #[test]
    fn skips_bindings_with_safe_visible_values() {
        let source = "\
#!/bin/bash
n=42
s=abc
glob='*'
split='1 2'
copy=\"$n\"
alias=$s
printf '%s\\n' $n $s $glob $split $copy $alias
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$glob", "$split"]
        );
    }

    #[test]
    fn skips_safe_literal_bindings_inside_nested_command_substitutions() {
        let source = "\
#!/bin/bash
URL=https://example.com/file.tgz
FILE=$(basename $URL)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_numeric_shell_variables() {
        let source = "\
#!/bin/bash
printf '%s\\n' $(ps -o comm= -p $PPID)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_reassigned_ppid_in_sh_mode() {
        let source = "\
#!/bin/sh
PPID='a b'
printf '%s\\n' $PPID
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PPID"]
        );
    }

    #[test]
    fn skips_safe_here_string_operands() {
        let source = "\
#!/bin/bash
URL=https://example.com/file.tgz
cat <<< $URL
cat <<< $PPID
v='a b'
cat <<< $v
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$v"]
        );
    }

    #[test]
    fn skips_safe_literal_loop_variables() {
        let source = "\
#!/bin/bash
for v in one two; do
  unset $v
done
for i in 16 32 64; do
  cmd ${i}x${i}! \"$i\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_loop_variables_derived_from_expanded_values() {
        let source = "\
#!/bin/bash
PRGNAM=neverball
BONUS=neverputt
for i in $PRGNAM $BONUS; do
  install -D ${i}.desktop /tmp/$i.png
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${i}", "$i"]
        );
    }

    #[test]
    fn reports_loop_variables_derived_from_at_slices() {
        let source = "\
#!/bin/bash
f() {
  for v in ${@:2}; do
    del $v
  done
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$v"]
        );
    }

    #[test]
    fn skips_direct_at_slices_that_belong_to_array_split_handling() {
        let source = "\
#!/bin/bash
f() {
  dns_set ${@:2}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_bindings_derived_from_arithmetic_values() {
        let source = "\
#!/bin/bash
x=$((1 + 2))
y=\"$x\"
z=${x}
printf '%s\\n' $x $y $z
if [ $x -eq 0 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_conditionally_sanitized_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
if [ \"$foo\" = \"\" ]; then
  foo=0
fi
if [ $foo -eq 1 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn reports_conditionally_initialized_bindings_with_unknown_fallbacks() {
        let source = "\
#!/bin/bash
if [ \"$1\" = yes ]; then
  foo=0
fi
printf '%s\\n' $foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn skips_straight_line_safe_overwrites_in_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
foo=0
if [ $foo -eq 1 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_case_arm_safe_overwrites_in_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
  settings)
    foo=0
    if [ $foo -eq 1 ]; then :; fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_case_arm_safe_overwrites_even_with_nested_conditional_updates() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
  settings)
    foo=1
    while [ $# -gt 1 ]; do
      shift
      case $1 in
        --no) foo=0 ;;
      esac
    done
    if [ $foo -eq 1 ]; then :; fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_if_else_safe_literal_bindings() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
free ${humanreadable}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_if_else_safe_literal_bindings_inside_command_substitutions() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
value=\"$(free ${humanreadable} | awk '{print $2}')\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_helper_initialized_option_flags_after_intermediate_calls() {
        let source = "\
#!/bin/bash
fn_select_compression() {
  if command -v zstd >/dev/null 2>&1; then
    compressflag=--zstd
  elif command -v pigz >/dev/null 2>&1; then
    compressflag=--use-compress-program=pigz
  elif command -v gzip >/dev/null 2>&1; then
    compressflag=--gzip
  else
    compressflag=
  fi
}

fn_backup_check_lockfile() { :; }
fn_backup_create_lockfile() { :; }
fn_backup_init() { :; }
fn_backup_stop_server() { :; }
fn_backup_dir() { :; }

fn_backup_compression() {
  if [ -n \"${compressflag}\" ]; then
    tar ${compressflag} -hcf out.tar ./.
  else
    tar -hcf out.tar ./.
  fi
}

fn_select_compression
fn_backup_check_lockfile
fn_backup_create_lockfile
fn_backup_init
fn_backup_stop_server
fn_backup_dir
fn_backup_compression
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_helper_initialized_bindings_when_other_callers_skip_the_helper() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  printf '%s\\n' ${flag}
}

safe_path() {
  init_flag
  render
}

unsafe_path() {
  render
}

safe_path
unsafe_path
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${flag}"]
        );
    }

    #[test]
    fn reports_helper_bindings_when_initializers_are_guarded_by_conditionals() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  if [ \"$1\" = yes ]; then
    init_flag
  fi
  printf '%s\\n' ${flag}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${flag}"]
        );
    }

    #[test]
    fn skips_helper_initialized_bindings_when_all_callers_provide_distinct_values() {
        let source = "\
#!/bin/bash
init_flag_a() {
  flag=-a
}

init_flag_b() {
  flag=-b
}

render() {
  printf '%s\\n' ${flag}
}

safe_path_a() {
  init_flag_a
  render
}

safe_path_b() {
  init_flag_b
  render
}

safe_path_a
safe_path_b
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_ambient_contract_bindings_without_known_values() {
        let path = Path::new("/tmp/void-packages/common/build-style/example.sh");
        let source = "\
#!/bin/sh
helper() {
  printf '%s\\n' $wrksrc $pkgname
}
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$wrksrc", "$pkgname"]
        );
    }

    #[test]
    fn skips_static_suffix_bindings_in_slackbuild_subshell_paths() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
if [ \"$ARCH\" = \"i386\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i486\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i586\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i686\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  MULTILIB=\"YES\"
  LIBDIRSUFFIX=\"64\"
elif [ \"$ARCH\" = \"armv7hl\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"s390\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
else
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
fi

if [ ${MULTILIB} = \"YES\" ]; then
  printf '%s\\n' multilib
fi

(
  ./configure \
    --libdir=/usr/lib${LIBDIRSUFFIX} \
    --with-python-dir=/lib${LIBDIRSUFFIX}/python2.7/site-packages \
    --with-java-home=/usr/lib${LIBDIRSUFFIX}/jvm/jre
)
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn keeps_safe_indirect_bindings_but_reports_parameter_operator_results() {
        let source = "\
#!/bin/bash
base=abc
name=base
upper=${base^^}
value='a b*'
quoted=${value@Q}
printf '%s\\n' ${!name} $upper $quoted
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$upper", "$quoted"]
        );
    }

    #[test]
    fn indirect_cycles_and_multi_field_targets_stay_unsafe() {
        let source = "\
#!/bin/bash
split='1 2'
name=split
a=$b
b=$a
printf '%s\\n' ${!name} $a
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!name}", "$a"]
        );
    }

    #[test]
    fn skips_plain_unquoted_scalars_in_native_zsh_mode() {
        let source = "print $name\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_unquoted_scalars_after_setopt_sh_word_split_in_zsh() {
        let source = "setopt sh_word_split\nprint $name\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn reports_zsh_force_split_modifier_even_without_sh_word_split() {
        let source = "print ${=name}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${=name}"]
        );
    }

    #[test]
    fn skips_zsh_double_tilde_modifier_when_it_forces_globbing_off() {
        let source = "print ${~~name}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
