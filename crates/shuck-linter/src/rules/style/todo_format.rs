use crate::{Checker, Rule, Violation};

pub struct TodoFormat {
    missing_owner: bool,
    missing_message: bool,
}

impl Violation for TodoFormat {
    fn rule() -> Rule {
        Rule::TodoFormat
    }

    fn message(&self) -> String {
        match (self.missing_owner, self.missing_message) {
            (true, true) => "add an owner and message to this action comment".to_owned(),
            (true, false) => "add an owner to this action comment".to_owned(),
            (false, true) => "add a message to this action comment".to_owned(),
            (false, false) => "action comment is complete".to_owned(),
        }
    }
}

pub fn todo_format(checker: &mut Checker) {
    let options = checker.rule_options().s082.clone();
    let diagnostics = checker
        .facts()
        .source_facts()
        .todo_comment_facts()
        .iter()
        .filter_map(|fact| {
            todo_comment_violation(fact.content(), &options)
                .map(|(kind, violation)| crate::Diagnostic::new(violation, fact.marker_span(kind)))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn todo_comment_violation<'a>(
    content: &'a str,
    options: &'a crate::S082RuleOptions,
) -> Option<(&'a str, TodoFormat)> {
    for kind in options.kinds.iter().filter(|kind| !kind.is_empty()) {
        let Some(rest) = marker_rest(content, kind) else {
            continue;
        };
        let owner_rest = owner_annotation_rest(rest);
        let has_owner = owner_rest.is_some();
        let missing_owner = options.require_owner && !has_owner;
        let message_rest = owner_rest.unwrap_or(rest);
        let missing_message = options.require_message && !has_non_empty_message(message_rest);
        if missing_owner || missing_message {
            return Some((
                kind.as_str(),
                TodoFormat {
                    missing_owner,
                    missing_message,
                },
            ));
        }
    }

    None
}

fn marker_rest<'a>(content: &'a str, kind: &str) -> Option<&'a str> {
    let rest = content.strip_prefix(kind)?;
    if rest
        .chars()
        .next()
        .is_some_and(|char| char.is_ascii_alphanumeric() || char == '_')
    {
        return None;
    }

    Some(rest)
}

fn owner_annotation_rest(rest: &str) -> Option<&str> {
    let after_open = rest.strip_prefix('(')?;
    let close = after_open.find(')')?;
    let owner = &after_open[..close];
    if owner.trim().is_empty() {
        return None;
    }

    Some(&after_open[close + 1..])
}

fn has_non_empty_message(rest: &str) -> bool {
    let message = rest.trim_start();
    let message = message.strip_prefix(':').unwrap_or(message).trim_start();
    !message.is_empty()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_missing_owner() {
        let source = "#!/bin/bash\n# TODO finish the parser case\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TodoFormat));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "add an owner to this action comment"
        );
        assert_eq!(diagnostics[0].span.slice(source), "TODO");
    }

    #[test]
    fn reports_missing_message() {
        let source = "#!/bin/bash\n# FIXME(alice):\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TodoFormat));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "add a message to this action comment"
        );
        assert_eq!(diagnostics[0].span.slice(source), "FIXME");
    }

    #[test]
    fn reports_missing_owner_and_message() {
        let source = "#!/bin/bash\n# XXX\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TodoFormat));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "add an owner and message to this action comment"
        );
        assert_eq!(diagnostics[0].span.slice(source), "XXX");
    }

    #[test]
    fn accepts_owner_and_message() {
        let source = "#!/bin/bash\n# TODO(alice): finish the parser case\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TodoFormat));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn configured_kind_is_case_sensitive() {
        let source = "#!/bin/bash\n# note(alice): finish the parser case\n# NOTE finish\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TodoFormat).with_s082_kinds(["NOTE".to_owned()]),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "NOTE");
    }

    #[test]
    fn can_disable_owner_requirement() {
        let source = "#!/bin/bash\n# TODO finish the parser case\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TodoFormat).with_s082_require_owner(false),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn can_disable_message_requirement() {
        let source = "#!/bin/bash\n# TODO(alice)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TodoFormat).with_s082_require_message(false),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_non_comment_text_and_longer_words() {
        let source = "#!/bin/bash\necho TODO\n# TODONOT(alice): leave this alone\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TodoFormat));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn checks_inline_comments() {
        let source = "#!/bin/bash\necho hi # TODO(alice)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TodoFormat));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "TODO");
    }
}
