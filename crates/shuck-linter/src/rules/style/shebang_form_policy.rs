use crate::facts::ShebangInvocationForm;
use crate::{Checker, Rule, Violation};

pub struct ShebangFormPolicy;

impl Violation for ShebangFormPolicy {
    fn rule() -> Rule {
        Rule::ShebangFormPolicy
    }

    fn message(&self) -> String {
        "shebang invocation form is outside the configured policy".to_owned()
    }
}

pub fn shebang_form_policy(checker: &mut Checker) {
    let Some(shebang) = checker.facts().source_facts().shebang_invocation() else {
        return;
    };

    let options = &checker.rule_options().s079;
    if !allowed_path_matches(shebang.text(), &options.allowed_paths)
        && !allowed_form_matches(shebang.form(), &options.allowed_forms)
    {
        checker.report(ShebangFormPolicy, shebang.span());
    }
}

fn allowed_path_matches(text: &str, allowed_paths: &[String]) -> bool {
    let text = text.trim();
    allowed_paths.iter().any(|allowed| allowed.trim() == text)
}

fn allowed_form_matches(form: ShebangInvocationForm, allowed_forms: &[String]) -> bool {
    allowed_forms
        .iter()
        .any(|allowed| allowed_form_name_matches(form, allowed))
}

fn allowed_form_name_matches(form: ShebangInvocationForm, allowed: &str) -> bool {
    match allowed.trim().to_ascii_lowercase().as_str() {
        "absolute-path" => form == ShebangInvocationForm::AbsolutePath,
        "env-lookup" => form == ShebangInvocationForm::EnvLookup,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_disallowed_absolute_path_forms() {
        let source = "#!/usr/local/bin/bash\necho hello\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ShebangFormPolicy));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["#!/usr/local/bin/bash"]
        );
    }

    #[test]
    fn accepts_default_env_form_and_default_exact_paths() {
        for source in [
            "#!/usr/bin/env bash\necho hello\n",
            "#!/bin/bash\necho hello\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::ShebangFormPolicy));

            assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
        }
    }

    #[test]
    fn accepts_configured_absolute_path_forms() {
        let source = "#!/usr/local/bin/bash\necho hello\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShebangFormPolicy)
                .with_s079_allowed_forms(["absolute-path"])
                .with_s079_allowed_paths(std::iter::empty::<&str>()),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn accepts_configured_exact_paths_without_matching_form() {
        let source = "#!/opt/project/bash\necho hello\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShebangFormPolicy)
                .with_s079_allowed_forms(["env-lookup"])
                .with_s079_allowed_paths(["/opt/project/bash"]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_files_without_a_parseable_shebang_invocation() {
        for source in ["echo hello\n", "# !/usr/local/bin/bash\necho hello\n"] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::ShebangFormPolicy));

            assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
        }
    }
}
