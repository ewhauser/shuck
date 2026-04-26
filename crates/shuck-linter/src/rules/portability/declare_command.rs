use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct DeclareCommand {
    portability_builtin_name: &'static str,
}

impl Violation for DeclareCommand {
    fn rule() -> Rule {
        Rule::DeclareCommand
    }

    fn message(&self) -> String {
        format!(
            "`{}` is not portable in `sh` scripts",
            self.portability_builtin_name
        )
    }
}

pub fn declare_command(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            portability_builtin_name(fact).map(|portability_builtin_name| {
                (
                    portability_builtin_name,
                    declaration_command_anchor_span(fact, source),
                )
            })
        })
        .collect::<Vec<_>>();

    for (portability_builtin_name, span) in diagnostics {
        checker.report(
            DeclareCommand {
                portability_builtin_name,
            },
            span,
        );
    }
}

fn portability_builtin_name(fact: crate::CommandFactRef<'_, '_>) -> Option<&'static str> {
    match fact.effective_or_literal_name()? {
        "declare" => Some("declare"),
        "typeset" => Some("typeset"),
        "shopt" => Some("shopt"),
        "complete" => Some("complete"),
        "compgen" => Some("compgen"),
        "caller" => Some("caller"),
        "dirs" => Some("dirs"),
        "disown" => Some("disown"),
        "suspend" => Some("suspend"),
        "mapfile" => Some("mapfile"),
        "readarray" => Some("readarray"),
        "pushd" => Some("pushd"),
        "popd" => Some("popd"),
        _ => None,
    }
}

fn declaration_command_anchor_span(fact: crate::CommandFactRef<'_, '_>, source: &str) -> Span {
    let start = declaration_command_anchor_start(fact, source);

    if let Some(declaration) = fact.declaration() {
        let end = declaration_anchor_end(fact, declaration.head_span.end, source);

        return Span::from_positions(start, end);
    }

    Span::from_positions(start, command_anchor_end(fact, source))
}

fn declaration_command_anchor_start(
    fact: crate::CommandFactRef<'_, '_>,
    source: &str,
) -> shuck_ast::Position {
    if !fact.wrappers().is_empty() {
        return fact.span().start;
    }

    if let Some(declaration) = fact.declaration() {
        return if declaration.assignments.is_empty() {
            declaration.head_span.start
        } else {
            fact.span().start
        };
    }

    effective_name_span(fact, source)
        .map(|span| span.start)
        .unwrap_or_else(|| fact.span().start)
}

fn effective_name_span(fact: crate::CommandFactRef<'_, '_>, source: &str) -> Option<Span> {
    let word = fact.body_name_word()?;
    let name = fact.effective_or_literal_name()?;
    let text = word.span.slice(source);
    let offset = text.rfind(name)?;

    (offset + name.len() == text.len()).then(|| {
        let start = word.span.start.advanced_by(&text[..offset]);
        Span::from_positions(start, word.span.end)
    })
}

fn declaration_anchor_end(
    fact: crate::CommandFactRef<'_, '_>,
    mut end: shuck_ast::Position,
    source: &str,
) -> shuck_ast::Position {
    for redirect in fact.redirect_facts() {
        if redirect.span().end.offset > end.offset {
            end = redirect.span().end;
        }
    }

    clip_terminator(fact, end, source)
}

fn command_anchor_end(fact: crate::CommandFactRef<'_, '_>, source: &str) -> shuck_ast::Position {
    let end = fact
        .shellcheck_command_span(source)
        .map(|span| span.end)
        .unwrap_or_else(|| fact.span_in_source(source).end);
    clip_terminator(fact, end, source)
}

fn clip_terminator(
    fact: crate::CommandFactRef<'_, '_>,
    mut end: shuck_ast::Position,
    _source: &str,
) -> shuck_ast::Position {
    if let Some(terminator_span) = fact.stmt_terminator_span()
        && terminator_span.start.offset < end.offset
    {
        end = terminator_span.start;
    }

    end
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn excludes_assignment_values_for_direct_declarations() {
        let source = "#!/bin/sh\nFOO=1 declare bar=baz\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "FOO=1 declare bar");
    }

    #[test]
    fn excludes_assignment_values_for_direct_typeset_declarations() {
        let source = "#!/bin/sh\nFOO=1 typeset bar=baz\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "`typeset` is not portable in `sh` scripts"
        );
        assert_eq!(diagnostics[0].span.slice(source), "FOO=1 typeset bar");
    }

    #[test]
    fn includes_attached_redirects_without_statement_terminators() {
        let source = "#!/bin/sh\nif declare -f pre_step >/dev/null; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "declare -f pre_step >/dev/null"
        );
    }

    #[test]
    fn anchors_wrapped_declare_on_the_full_command() {
        let source = "#!/bin/sh\ncommand declare wrapped=value\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "command declare wrapped=value"
        );
    }

    #[test]
    fn anchors_wrapped_typeset_on_the_full_command() {
        let source = "#!/bin/sh\ncommand typeset wrapped=value\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "`typeset` is not portable in `sh` scripts"
        );
        assert_eq!(
            diagnostics[0].span.slice(source),
            "command typeset wrapped=value"
        );
    }

    #[test]
    fn keeps_declaration_operands_after_interleaved_redirects() {
        let source = "#!/bin/sh\ndeclare >/tmp/out foo=bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "declare >/tmp/out foo");
    }

    #[test]
    fn excludes_leading_backslashes_from_escaped_typeset_command_spans() {
        let source = "#!/bin/sh\n  \\typeset foo=bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "`typeset` is not portable in `sh` scripts"
        );
        assert_eq!(diagnostics[0].span.slice(source), "typeset foo=bar");
    }

    #[test]
    fn reports_other_non_portable_builtins_in_sh_scripts() {
        let source = "\
#!/bin/sh
shopt -s nullglob
complete -F _portable portable
compgen -A file
caller
dirs
disown
suspend
mapfile entries
readarray lines
pushd /tmp
popd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>(),
            vec![
                "`shopt` is not portable in `sh` scripts",
                "`complete` is not portable in `sh` scripts",
                "`compgen` is not portable in `sh` scripts",
                "`caller` is not portable in `sh` scripts",
                "`dirs` is not portable in `sh` scripts",
                "`disown` is not portable in `sh` scripts",
                "`suspend` is not portable in `sh` scripts",
                "`mapfile` is not portable in `sh` scripts",
                "`readarray` is not portable in `sh` scripts",
                "`pushd` is not portable in `sh` scripts",
                "`popd` is not portable in `sh` scripts",
            ]
        );
    }

    #[test]
    fn clips_non_declaration_builtins_before_statement_terminators() {
        let source = "#!/bin/sh\nif shopt -q login_shell; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "shopt -q login_shell");
    }

    #[test]
    fn excludes_inline_comments_from_non_declaration_builtin_spans() {
        let source = "#!/bin/sh\npopd # restore stack\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "popd");
    }
}
