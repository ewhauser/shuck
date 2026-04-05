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
        .map(|overwritten| {
            let span = checker.semantic().binding(overwritten.first).span;
            (overwritten.name.to_string(), span)
        })
        .collect::<Vec<_>>();

    for (name, span) in overwritten {
        checker.report(OverwrittenFunction { name }, span);
    }
}
