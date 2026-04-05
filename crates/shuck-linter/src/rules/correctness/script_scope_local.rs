use shuck_semantic::{DeclarationBuiltin, ScopeKind};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct LocalTopLevel;

impl Violation for LocalTopLevel {
    fn rule() -> Rule {
        Rule::LocalTopLevel
    }

    fn message(&self) -> String {
        "`local` is only valid inside a function body".to_owned()
    }
}

pub fn local_top_level(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Dash) {
        return;
    }

    for declaration in checker.semantic().declarations() {
        if declaration.builtin != DeclarationBuiltin::Local {
            continue;
        }

        let scope = checker.semantic().scope_at(declaration.span.start.offset);
        let inside_function = checker
            .semantic()
            .ancestor_scopes(scope)
            .any(|scope| matches!(checker.semantic().scope_kind(scope), ScopeKind::Function(_)));

        if inside_function {
            continue;
        }

        checker.report(LocalTopLevel, declaration.span);
    }
}
