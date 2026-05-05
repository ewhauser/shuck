use crate::{Checker, PlainUnindexedArrayReferenceFact, Rule, Violation};

pub struct QuotedBashSource;

impl Violation for QuotedBashSource {
    fn rule() -> Rule {
        Rule::QuotedBashSource
    }

    fn message(&self) -> String {
        "array references should choose an explicit element or selector".to_owned()
    }
}

pub fn quoted_bash_source(checker: &mut Checker) {
    let spans = checker
        .facts()
        .plain_unindexed_array_references()
        .filter_map(|fact| match fact {
            PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
                Some(reference.diagnostic_span())
            }
            PlainUnindexedArrayReferenceFact::NativeZshScalar(_) => None,
            PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
                Some(reference.diagnostic_span())
            }
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedBashSource);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect, lint_file_at_path};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn reports_plain_unindexed_array_references() {
        let source = "\
#!/bin/bash
arr=(one two)
declare -A map=([key]=value)
read -ra read_items
mapfile map_items
x=\"$BASH_SOURCE\"
y=\"${BASH_SOURCE}\"
printf '%s\\n' $arr \"${arr}\" pre${arr}post \"$map\" \"$read_items\" \"$map_items\"
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
if [[ \"$BASH_SOURCE\" == foo ]]; then :; fi
for item in \"$BASH_SOURCE\"; do
  :
done
cat <<EOF
$arr
${arr}
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$BASH_SOURCE",
                "${BASH_SOURCE}",
                "$arr",
                "${arr}",
                "${arr}",
                "$map",
                "$read_items",
                "$map_items",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$arr",
                "${arr}",
            ]
        );
    }

    #[test]
    fn ignores_scalar_indexed_selector_and_non_access_forms() {
        let source = "\
#!/bin/bash
name=scalar
MAPFILE=scalar
arr=(one two)
x=$BASH_SOURCE
y=${BASH_SOURCE}
z=\"${BASH_SOURCE[0]}\"
q=\"${BASH_SOURCE[@]}\"
r=\"${BASH_SOURCE[*]}\"
s=\"${BASH_SOURCE%/*}\"
t=\"${BASH_SOURCE:-fallback}\"
v=\"${BASH_SOURCE-}\"
u=\"\\$BASH_SOURCE\"
printf '%s\\n' \"$name\" \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${arr%one}\" \"${arr:-fallback}\"
only_declared() {
  local -a local_array
  printf '%s\\n' \"$local_array\"
}
for item in \"$@\"; do
  item=($item)
done
read -ra read_items <<<\"$read_items\"
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE", "${BASH_SOURCE}", "$MAPFILE"]
        );
    }

    #[test]
    fn zsh_scalar_string_slices_do_not_create_array_reference_history() {
        let source = "\
#!/bin/zsh
opt=ksh_arrays
setopt \"$opt\"
ret=abcdef
ret[-1,-1]=''
ret=${ret[2,-1]}
printf '%s\\n' \"$ret\" \"${ret}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn zsh_array_presence_tests_do_not_require_explicit_selectors() {
        let source = "\
#!/bin/zsh
opt=ksh_arrays
setopt \"$opt\"
precm=(builtin emulate zsh)
[[ -n $precm ]] && builtin ${precm[@]} 'source \"$ZERO\"'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
for item in $filelist; do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nested_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
tests=\"$(for item in $filelist; do
  :
done)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn string_binary_conditions_do_not_count_as_presence_guards() {
        let source = "\
#!/bin/bash
apt_pkgs=()
for pkg in \"$@\"; do
  pkg=(one two three)
  if [[ \"${pkg[0]}\" == one ]]; then
    :
  fi
  if hasPackage \"$pkg\"; then
    apt_pkgs+=(\"$pkg\")
  fi
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pkg", "$pkg"]
        );
    }

    #[test]
    fn unset_does_not_reset_array_type() {
        let source = "\
#!/bin/bash
cleared_array=(one two)
unset cleared_array
cleared_array=scalar
printf '%s\\n' \"$cleared_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$cleared_array"]
        );
    }

    #[test]
    fn target_rebindings_reset_inherited_array_type() {
        let source = "\
#!/bin/bash
loop_value=(one two)
for loop_value in one two; do
  printf '%s\\n' \"$loop_value\"
done
read_value=(one two)
read read_value <<<input
printf '%s\\n' \"$read_value\"
printf_value=(one two)
printf -v printf_value '%s' input
printf '%s\\n' \"$printf_value\"
local_reset() {
  local local_value=(one two)
  local local_value
  printf '%s\\n' \"$local_value\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unbound_runtime_arrays_without_bash_prelude() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$BASH_SOURCE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE"]
        );
    }

    #[test]
    fn reports_runtime_array_names_even_after_scalar_rebinding() {
        let source = "\
#!/bin/bash
MAPFILE=scalar
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$MAPFILE"]
        );
    }

    #[test]
    fn array_declarations_stay_sticky_through_plain_assignments() {
        let source = "\
#!/bin/bash
declare -a additional_packages
additional_packages=$1
split_string ${additional_packages}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${additional_packages}"]
        );
    }

    #[test]
    fn later_presence_guards_only_suppress_the_same_binding() {
        let source = "\
#!/bin/bash
foo=scalar
[ -n \"$foo\" ]
foo=(one two)
printf '%s\\n' \"$foo\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn variable_set_presence_guards_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
[[ -v arr ]]
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn variable_set_presence_guards_do_not_cross_rebindings() {
        let source = "\
#!/bin/bash
arr=scalar
[[ -v arr ]]
arr=(one two)
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn prior_presence_guards_in_sibling_case_arms_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
f() {
  local dir
  case \"$1\" in
    up) dir=(\"Up\");;
  esac
  case \"$2\" in
    hat)
      [[ -n \"$dir\" ]]
      ;;
    *)
      [[ \"$dir\" == \"Up\" || \"$dir\" == \"Left\" ]]
      ;;
  esac
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir"]
        );
    }

    #[test]
    fn attribute_only_declarations_keep_array_type() {
        let source = "\
#!/bin/bash
arr=(one two)
readonly arr
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn function_local_declare_arrays_still_warn() {
        let source = "\
#!/bin/bash
f() {
  declare -a items
  printf '%s\\n' \"$items\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$items"]
        );
    }

    #[test]
    fn nested_command_substitution_presence_tests_do_not_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
[ -n \"$(printf '%s' \"$arr\")\" ]
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr", "$arr"]
        );
    }

    #[test]
    fn presence_tests_inside_command_substitutions_suppress_later_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
out=$( [ -n \"$arr\" ]; printf x )
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn same_command_prefix_array_assignments_still_warn() {
        let source = "\
#!/bin/bash
arr=(old1 old2)
arr=(new1 new2) printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn read_option_values_do_not_become_array_targets() {
        let source = "\
#!/bin/bash
delimiter=:
read -d delimiter -a arr <<<\":\"
printf '%s\\n' \"$delimiter\"
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn mapfile_option_values_do_not_become_array_targets() {
        let source = "\
#!/bin/bash
callback=scalar
mapfile -C callback -c 1 lines < <(printf '%s\\n' value)
printf '%s\\n' \"$callback\"
printf '%s\\n' \"$lines\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$lines"]
        );
    }

    #[test]
    fn local_scalar_assignments_do_not_inherit_outer_array_bindings() {
        let source = "\
#!/bin/bash
declare -a ids
ids=()
set_to_liked() {
  local ids
  { local IFS=','; ids=\"$*\"; }
  if [ -z \"$ids\" ]; then
    return
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn printf_targets_after_local_declarations_do_not_inherit_outer_arrays() {
        let source = "\
#!/bin/bash
args=(\"$@\")
f() {
  local args
  printf -v args '%q ' \"$@\"
  printf '%s\\n' \"$args\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn local_append_declarations_keep_array_type() {
        let source = "\
#!/bin/bash
f() {
  local DOKKU_LOGS_CMD=()
  DOKKU_LOGS_CMD+=\"(cmd)\"
  local DOKKU_LOGS_CMD+=\"; \"
  bash -c \"($DOKKU_LOGS_CMD)\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$DOKKU_LOGS_CMD"]
        );
    }

    #[test]
    fn ignores_references_inside_own_array_assignment() {
        let source = "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_references_inside_same_name_array_readers() {
        let source = "\
#!/bin/bash
read -r -a key_value <<<\"$(printf '%s\\n' \"$key_value\")\"
mapfile -t ports_configured < <(printf '%s\\n' \"${ports_configured}\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_bindings_reset_inherited_array_type() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(. ./helper.sh; printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
",
        )
        .unwrap();
        fs::write(&helper, "TERMUX_PKG_VERSION=helper\n").unwrap();

        let source = fs::read_to_string(&main).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let diagnostics = lint_file_at_path(
            &output.file,
            &source,
            &indexer,
            &LinterSettings::for_rule(Rule::QuotedBashSource),
            None,
            Some(&main),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn follows_prior_visible_array_bindings() {
        let source = "\
#!/bin/bash
before_use() {
  printf '%s\\n' \"$future_array\"
}
future_array=(one two)
after_use() {
  printf '%s\\n' \"$future_array\"
}
former_array=(one two)
former_array=scalar
printf '%s\\n' \"$former_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$future_array", "$former_array"]
        );
    }

    #[test]
    fn follows_prior_array_bindings_by_source_order() {
        let source = "\
#!/bin/bash
first_function() {
  target=(one two)
}
second_function() {
  local target=$1
  printf '%s\\n' \"$target\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn zsh_initialized_local_scalar_rebindings_do_not_inherit_outer_array_type() {
        let source = "\
#!/bin/zsh
cmd=(curl -I)
f() {
  local cmd=cp
  eval \"$cmd\"
  local ice_key=\"$cmd\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn zsh_initialized_local_scalar_rebindings_suppress_nested_subshell_refs() {
        let source = "\
#!/bin/zsh
cmd=(curl -I)
f() {
  local cmd=cp
  (
    command $cmd -f src dst
  )
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn zsh_later_array_bindings_after_scalar_local_barriers_stay_clean() {
        let source = "\
#!/bin/zsh
items=(old)
f() {
  local items=scalar
  items=(new)
  print -r -- $items
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn zsh_plain_array_and_assoc_scalar_expansions_are_allowed() {
        let source = "\
#!/bin/zsh
local -a usage
usage=(one two)
print -l -- $usage

local -aU pats
pats=(a b)
for pat in $pats; do :; done

DECOMPRESSCMD=( unxz )
[[ $DECOMPRESSCMD != /* ]]

local -A OPTS
OPTS[k]=1
[[ -n $OPTS && -n ${OPTS[k]} ]]

local -a ___opt
___opt=(-a -b)
.zinit-load-snippet $___opt foo
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn zsh_non_effect_mentions_of_ksh_arrays_do_not_disable_native_scalar_policy() {
        let source = "\
#!/bin/zsh
# emulate ksh
note='setopt ksh_arrays'
arr=(one two)
print -r -- $arr
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn zsh_emulate_local_function_shape_keeps_plain_array_expansions_clean() {
        let source = "\
#!/bin/zsh
f() {
  emulate -LR zsh ${=${options[xtrace]:#off}:+-o xtrace}
  setopt extendedglob warncreateglobal typesetsilent noshortloops rcquotes
  local id_as=$1 plugin_dir
  local -A ICE
  if [[ -n \"${ICE[compile]}\" ]]; then
    local -aU pats list=()
    pats=(${(s.;.)ICE[compile]})
    local pat
    for pat in $pats; do
      list+=(\"${plugin_dir:A}/\"${~pat}(.N))
    done
  fi
}
g() {
  emulate -LR ksh
  local -a other=(one two)
  print -r -- $other
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$other"]
        );
    }

    #[test]
    fn zsh_conditional_function_local_disable_still_warns_in_ambiguous_ksh_context() {
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
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn zsh_ksh_array_mode_keeps_plain_array_reference_warnings() {
        let source = "\
#!/bin/zsh
setopt ksh_arrays
arr=(one two)
print -r -- $arr

emulate ksh
other=(three four)
print -r -- $other

emulate -L ksh
flagged=(five six)
print -r -- $flagged

f() {
  indirect=(seven eight)
  print -r -- $indirect
}
fn=f
setopt ksh_arrays
$fn
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr", "$other", "$flagged", "$indirect"]
        );
    }

    #[test]
    fn zsh_dynamic_ksh_array_option_names_keep_plain_array_reference_warnings() {
        for source in [
            "\
#!/bin/zsh
opt=ksh_arrays
setopt \"$opt\"
arr=(one two)
print -r -- $arr
",
            "\
#!/bin/zsh
opt=no_ksh_arrays
unsetopt \"$opt\"
arr=(one two)
print -r -- $arr
",
            "\
#!/bin/zsh
mode=ksh
emulate \"$mode\"
arr=(one two)
print -r -- $arr
",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
            );

            assert_eq!(
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$arr"],
                "{source}"
            );
        }
    }

    #[test]
    fn zsh_pattern_ksh_array_option_names_keep_plain_array_reference_warnings() {
        for source in [
            "\
#!/bin/zsh
setopt -m 'ksh*'
arr=(one two)
print -r -- $arr
",
            "\
#!/bin/zsh
unsetopt -m 'no_ksh*'
arr=(one two)
print -r -- $arr
",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
            );

            assert_eq!(
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$arr"],
                "{source}"
            );
        }
    }

    #[test]
    fn zsh_double_dash_ksh_array_option_names_keep_plain_array_reference_warnings() {
        for source in [
            "\
#!/bin/zsh
setopt -- ksh_arrays
arr=(one two)
print -r -- $arr
",
            "\
#!/bin/zsh
unsetopt -- no_ksh_arrays
arr=(one two)
print -r -- $arr
",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
            );

            assert_eq!(
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$arr"],
                "{source}"
            );
        }
    }

    #[test]
    fn zsh_top_level_indirect_ksh_mode_call_keeps_plain_array_reference_warnings() {
        let source = "\
#!/bin/zsh
f() {
  emulate ksh
}
dispatcher=f
$dispatcher
arr=(one two)
print -r -- $arr
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn zsh_unresolved_dispatch_can_still_keep_plain_array_reference_warnings() {
        let source = "\
#!/bin/zsh
enable_ksh() {
  emulate ksh
}
$dispatcher
arr=(one two)
print -r -- $arr
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn zsh_unknown_dispatcher_binding_can_still_keep_plain_array_reference_warnings() {
        let source = "\
#!/bin/zsh
enable_ksh() {
  emulate ksh
}
run_dispatcher() {
  unsetopt ksh_arrays
  dispatcher=$1
  $dispatcher
  arr=(one two)
  print -r -- $arr
}
run_dispatcher enable_ksh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn zsh_wrapped_top_level_indirect_ksh_mode_keeps_plain_array_reference_warnings() {
        let source = "\
#!/bin/zsh
f() {
  emulate ksh
}
dispatcher=f
noglob $dispatcher
arr=(one two)
print -r -- $arr
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn zsh_top_level_explicit_disable_after_indirect_ksh_call_restores_native_scalar_policy() {
        let source = "\
#!/bin/zsh
f() {
  emulate ksh
}
dispatcher=f
$dispatcher
unsetopt ksh_arrays
arr=(one two)
print -r -- $arr
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashSource).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_runtime_arrays_inside_assign_default_and_error_operands() {
        let source = "\
#!/bin/bash
: ${PROG:=$(basename ${BASH_SOURCE})}
local PATTERN=${2:?$FUNCNAME: a pattern is required}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${BASH_SOURCE}", "$FUNCNAME"]
        );
    }
}
