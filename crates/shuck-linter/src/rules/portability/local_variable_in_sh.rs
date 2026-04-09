use crate::{Checker, DeclarationKind, Rule, ShellDialect, Violation};

pub struct LocalVariableInSh;

impl Violation for LocalVariableInSh {
    fn rule() -> Rule {
        Rule::LocalVariableInSh
    }

    fn message(&self) -> String {
        "`local` is not portable in `sh` scripts".to_owned()
    }
}

pub fn local_variable_in_sh(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Sh {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let declaration = fact.declaration()?;
            (declaration.kind == DeclarationKind::Local).then_some(declaration.head_span)
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || LocalVariableInSh);
}
