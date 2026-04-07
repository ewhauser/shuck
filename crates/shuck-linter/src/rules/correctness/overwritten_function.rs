use shuck_semantic::{OverwrittenFunction as SemanticOverwrittenFunction, ScopeKind};

use crate::context::FileContextTag;
use crate::{Checker, Rule, Violation};

pub struct OverwrittenFunction {
    pub name: String,
}

impl Violation for OverwrittenFunction {
    fn rule() -> Rule {
        Rule::OverwrittenFunction
    }

    fn message(&self) -> String {
        format!(
            "function `{}` is overwritten before it can be called",
            self.name
        )
    }
}

pub fn overwritten_function(checker: &mut Checker) {
    let overwritten = checker
        .semantic()
        .call_graph()
        .overwritten
        .iter()
        .filter(|overwritten| !overwritten.first_called)
        .filter(|overwritten| !should_suppress_overwrite(checker, overwritten))
        .map(|overwritten| {
            let span = checker.semantic().binding(overwritten.first).span;
            (overwritten.name.to_string(), span)
        })
        .collect::<Vec<_>>();

    for (name, span) in overwritten {
        checker.report(OverwrittenFunction { name }, span);
    }
}

fn should_suppress_overwrite(
    checker: &Checker<'_>,
    overwritten: &SemanticOverwrittenFunction,
) -> bool {
    let file_context = checker.file_context();
    let first = checker.semantic().binding(overwritten.first);
    let second = checker.semantic().binding(overwritten.second);

    if file_context.has_tag(FileContextTag::ShellSpec)
        && !matches!(checker.semantic().scope_kind(first.scope), ScopeKind::File)
        && !matches!(checker.semantic().scope_kind(second.scope), ScopeKind::File)
    {
        return true;
    }

    (file_context.has_tag(FileContextTag::TestHarness)
        || file_context.has_tag(FileContextTag::HelperLibrary))
        && unset_function_between(
            checker,
            overwritten.name.as_str(),
            first.span.end.offset,
            second.span.start.offset,
        )
}

fn unset_function_between(
    checker: &Checker<'_>,
    name: &str,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.effective_name_is("unset")
            && fact.body_span().start.offset > start_offset
            && fact.body_span().start.offset < end_offset
            && fact
                .options()
                .unset()
                .is_some_and(|unset| unset.targets_function_name(checker.source(), name))
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn shellspec_nested_helper_factories_are_suppressed() {
        let source = "\
Describe 'matcher'
factory() {
  shellspec_matcher__match() { :; }
  shellspec_matcher__match() { :; }
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__matcher_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_double_swaps_after_unset_are_suppressed() {
        let source = "\
curl() { printf '%s\\n' first; }
unset -f curl
curl() { printf '%s\\n' second; }
curl
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/tests/nvm_compare_checksum_test.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ordinary_overwrites_still_report() {
        let source = "\
myfunc() { return 1; }
myfunc() { return 0; }
myfunc
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
    }

    #[test]
    fn plain_unset_does_not_suppress_function_overwrites() {
        let source = "\
curl() { printf '%s\\n' first; }
unset curl
curl() { printf '%s\\n' second; }
curl
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/tests/nvm_compare_checksum_test.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
    }
}
