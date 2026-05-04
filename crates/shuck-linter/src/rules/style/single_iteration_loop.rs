use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

pub struct SingleIterationLoop;

impl Violation for SingleIterationLoop {
    fn rule() -> Rule {
        Rule::SingleIterationLoop
    }

    fn message(&self) -> String {
        "this `for` loop iterates over a single item".to_owned()
    }
}

pub fn single_iteration_loop(checker: &mut Checker) {
    let locator = checker.locator();
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .filter(|header| !header.is_nested_word_command())
        .filter_map(|header| {
            let [word] = header.words() else {
                return None;
            };

            let fact = checker.facts().word_fact(
                word.span(),
                WordFactContext::Expansion(ExpansionContext::ForList),
            )?;
            fact.is_single_for_list_item(locator).then_some(word.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SingleIterationLoop);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_only_single_literal_for_list_items() {
        let source = "\
#!/bin/bash
set -- a b
for item in a; do
\tprintf '%s\\n' \"$item\"
done
for item in *.txt; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$@\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${@}\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${@:1}\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${@:-.}\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$PATCHES\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${dir}\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$(printf a)\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${dir}\"/x.patch; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${dir}\"/*; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$(printf /tmp)\"/x.patch; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$(dirname \"$0\")\"/../docs/usage/distrobox*; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$basedir/\"*; do
\tprintf '%s\\n' \"$item\"
done
for item in foo${bar}baz; do
\tprintf '%s\\n' \"$item\"
done
for item in ~; do
\tprintf '%s\\n' \"$item\"
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SingleIterationLoop));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a", "\"${dir}\"/x.patch", "\"$(printf /tmp)\"/x.patch", "~"]
        );
    }

    #[test]
    fn skips_zsh_for_list_scalars_when_glob_subst_can_fan_out() {
        let source = "\
setopt glob_subst
for item in $name; do
\tprint -r -- \"$item\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SingleIterationLoop).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_zsh_for_list_scalars_when_dynamic_glob_subst_is_ambiguous() {
        let source = "\
opt=glob_subst
setopt \"$opt\"
for item in $name; do
\tprint -r -- \"$item\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SingleIterationLoop).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_for_list_scalars_when_no_glob_masks_glob_subst() {
        let source = "\
setopt glob_subst no_glob
for item in $name; do
\tprint -r -- \"$item\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SingleIterationLoop).with_shell(ShellDialect::Zsh),
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
    fn zsh_array_fanout_comes_from_word_facts() {
        let source = "\
arr=(a b)
for item in ${arr}x; do
\tprint -r -- $item
done
setopt ksh_arrays
for item in ${arr}x; do
\tprint -r -- $item
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SingleIterationLoop).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr}x"]
        );
    }
}
