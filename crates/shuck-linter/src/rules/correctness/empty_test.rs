use shuck_ast::Command;

use crate::context::ContextRegionKind;
use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

use super::syntax::simple_test_operands;

pub struct EmptyTest;

impl Violation for EmptyTest {
    fn rule() -> Rule {
        Rule::EmptyTest
    }

    fn message(&self) -> String {
        "test expression is empty".to_owned()
    }
}

pub fn empty_test(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        checker.source(),
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::Simple(command) = command else {
                return;
            };

            if checker
                .file_context()
                .span_intersects_kind(ContextRegionKind::ShellSpecParametersBlock, command.span)
            {
                return;
            }

            if simple_test_operands(command, source).is_some_and(|operands| operands.is_empty()) {
                spans.push(command.span);
            }
        },
    );

    for span in spans {
        checker.report(EmptyTest, span);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn shellspec_parameters_blocks_are_ignored() {
        let source = "\
Describe 'clone'
Parameters
  \"test\"
  \"test$SHELLSPEC_LF\"
End

test
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__clone_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::EmptyTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::EmptyTest);
        assert_eq!(diagnostics[0].span.slice(source).trim_end(), "test");
    }
}
