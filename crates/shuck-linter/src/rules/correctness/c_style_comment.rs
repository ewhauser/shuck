use crate::context::FileContextTag;
use crate::{Checker, Rule, Violation};

pub struct CStyleComment;

impl Violation for CStyleComment {
    fn rule() -> Rule {
        Rule::CStyleComment
    }

    fn message(&self) -> String {
        "C-style comment syntax is not valid shell syntax".to_owned()
    }
}

pub fn c_style_comment(checker: &mut Checker) {
    if checker.file_context().has_tag(FileContextTag::PatchFile) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| {
            let name = command.body_name_word()?;
            name.span
                .slice(checker.source())
                .starts_with("/*")
                .then_some(name.span)
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || CStyleComment);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::test_snippet;
    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_c_style_comment_tokens() {
        let source = "#!/bin/sh\n/* note */\n/*compact*/\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleComment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "/*");
        assert_eq!(diagnostics[1].span.slice(source), "/*compact*/");
    }

    #[test]
    fn ignores_quoted_comment_like_text() {
        let source = "#!/bin/sh\necho '/* note */'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleComment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_patch_file_context() {
        let source = "/* Find the appropriate server to reach an ip */\n";
        let diagnostics = test_snippet_at_path(
            Path::new("change.patch"),
            source,
            &LinterSettings::for_rule(Rule::CStyleComment),
        );

        assert!(diagnostics.is_empty());
    }
}
