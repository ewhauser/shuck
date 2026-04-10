use crate::{Checker, Rule, Violation};

pub struct BrokenAssocKey;

impl Violation for BrokenAssocKey {
    fn rule() -> Rule {
        Rule::BrokenAssocKey
    }

    fn message(&self) -> String {
        "associative array keys in compound assignments need a closing `]` before `=`".to_owned()
    }
}

pub fn broken_assoc_key(checker: &mut Checker) {
    checker.report_all_dedup(checker.facts().broken_assoc_key_spans().to_vec(), || {
        BrokenAssocKey
    });
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_broken_assoc_keys_in_compound_assignments() {
        let source = "\
#!/bin/bash
declare -A table=([left]=1 [right=2)
other=([ok]=1 [broken=2)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenAssocKey));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[right=2", "[broken=2"]
        );
    }

    #[test]
    fn ignores_valid_or_non_associative_key_forms() {
        let source = "\
#!/bin/bash
declare -A table=([left]=1 [right]=2)
declare -a nums=([0]=1 [1=2)
declare -A pairs=(left one right two)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenAssocKey));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
