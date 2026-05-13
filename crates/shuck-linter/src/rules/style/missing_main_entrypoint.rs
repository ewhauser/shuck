use shuck_ast::Command;

use crate::{Checker, Rule, Violation};

pub struct MissingMainEntrypoint {
    main_name: String,
}

impl Violation for MissingMainEntrypoint {
    fn rule() -> Rule {
        Rule::MissingMainEntrypoint
    }

    fn message(&self) -> String {
        format!(
            "end non-trivial scripts with a call to `{}`",
            self.main_name
        )
    }
}

pub fn missing_main_entrypoint(checker: &mut Checker) {
    let options = checker.rule_options().s085.clone();
    let facts = checker.facts();
    let source = facts.source_facts().source();
    let command_facts = facts.command_facts();

    if source.lines().count() < options.non_trivial_line_threshold
        || command_facts.function_headers().len() < options.non_trivial_function_count
    {
        return;
    }

    let Some(top_level_body_span) = command_facts
        .statement_facts()
        .iter()
        .filter_map(|statement| {
            let command = command_facts.command(statement.command_id());
            command
                .enclosing_function_scope()
                .is_none()
                .then_some(statement.body_span())
        })
        .max_by_key(|span| span.end.offset.saturating_sub(span.start.offset))
    else {
        return;
    };

    let Some(last_statement) = command_facts
        .statement_facts()
        .iter()
        .filter(|statement| statement.body_span() == top_level_body_span)
        .max_by_key(|statement| statement.stmt_span().end.offset)
    else {
        return;
    };

    let last_command = command_facts.command(last_statement.command_id());
    let ends_with_main_call = matches!(last_command.command(), Command::Simple(_))
        && last_command.effective_name_is(&options.main_name);

    if ends_with_main_call {
        return;
    }

    let span = last_command
        .body_name_word()
        .map(|word| word.span)
        .unwrap_or_else(|| last_command.span_in_source(source));
    checker.report(
        MissingMainEntrypoint {
            main_name: options.main_name,
        },
        span,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    fn settings() -> LinterSettings {
        LinterSettings::for_rule(Rule::MissingMainEntrypoint)
            .with_s085_non_trivial_line_threshold(1)
            .with_s085_non_trivial_function_count(2)
    }

    #[test]
    fn reports_non_trivial_script_without_final_main_call() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { :; }
setup
run
";
        let diagnostics = test_snippet(source, &settings());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "run");
    }

    #[test]
    fn accepts_final_main_call() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { :; }
main() {
  setup
  run
}
main \"$@\"
";
        let diagnostics = test_snippet(source, &settings());

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_scripts_below_line_threshold() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { :; }
setup
run
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingMainEntrypoint)
                .with_s085_non_trivial_line_threshold(10)
                .with_s085_non_trivial_function_count(2),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_scripts_below_function_threshold() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { :; }
setup
run
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingMainEntrypoint)
                .with_s085_non_trivial_line_threshold(1)
                .with_s085_non_trivial_function_count(3),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn accepts_custom_entrypoint_name() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { setup; }
run \"$@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingMainEntrypoint)
                .with_s085_non_trivial_line_threshold(1)
                .with_s085_non_trivial_function_count(2)
                .with_s085_main_name("run"),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_statement_after_main_call() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { :; }
main() { run; }
main \"$@\"
echo done
";
        let diagnostics = test_snippet(source, &settings());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "echo");
    }

    #[test]
    fn function_definition_is_not_a_final_call() {
        let source = "\
#!/bin/bash
setup() { :; }
main() { setup; }
";
        let diagnostics = test_snippet(source, &settings());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "main() { setup; }");
    }

    #[test]
    fn nested_main_call_does_not_satisfy_top_level_entrypoint() {
        let source = "\
#!/bin/bash
setup() { :; }
run() { :; }
if true; then
  main \"$@\"
fi
";
        let diagnostics = test_snippet(source, &settings());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "if true; then\n  main \"$@\"\nfi"
        );
    }
}
