use crate::{Checker, Rule, ShellDialect, Violation};

pub struct TautologyChain;

impl Violation for TautologyChain {
    fn rule() -> Rule {
        Rule::TautologyChain
    }

    fn message(&self) -> String {
        "these alternatives make the OR chain unable to fail".to_owned()
    }
}

pub fn tautology_chain(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let spans = checker
        .facts()
        .command_facts()
        .tautology_chain_operator_spans()
        .to_vec();
    checker.report_all_dedup(spans, || TautologyChain);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_operator_before_the_later_conflicting_test() {
        let source = "\
#!/bin/bash
[[ a != \"$x\" ]] || [[ b != \"$x\" ]]
[ \"$n\" -ne 1 ] || [ 2 -ne \"$n\" ]
[[ a != \"$x\" ]] || maybe || [[ b != \"$x\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TautologyChain));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["||", "||", "||"]
        );
    }

    #[test]
    fn ignores_non_tautological_and_oracle_incompatible_shapes() {
        let source = "\
#!/bin/bash
[[ \"$x\" == a ]] || [[ \"$x\" == b ]]
[[ \"$x\" != a ]] || [[ \"$y\" != b ]]
[[ \"$x\" != a ]] || [[ \"$x\" != a ]]
[[ a != \"$x\" ]] || [[ b != $x ]]
[[ \"$x\" != a* ]] || [[ \"$x\" != b* ]]
test \"$x\" != a || test \"$x\" != b
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TautologyChain));

        assert!(diagnostics.is_empty());
    }
}
