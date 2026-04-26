use std::path::Path;

use crate::ShellDialect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileContextTag {
    ProjectClosure,
    DirectiveHandling,
}

impl FileContextTag {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProjectClosure => "project-closure",
            Self::DirectiveHandling => "directive-handling",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileContext {
    tags: Vec<FileContextTag>,
}

impl FileContext {
    pub fn new(tags: Vec<FileContextTag>) -> Self {
        let mut tags = tags;
        tags.sort_unstable();
        tags.dedup();

        Self { tags }
    }

    pub fn tags(&self) -> &[FileContextTag] {
        &self.tags
    }

    pub fn has_tag(&self, tag: FileContextTag) -> bool {
        self.tags.contains(&tag)
    }
}

pub fn classify_file_context(
    source: &str,
    _path: Option<&Path>,
    _shell: ShellDialect,
) -> FileContext {
    let lines = collect_lines(source);

    let mut tags = Vec::new();

    if lines.iter().any(|line| {
        let trimmed = line.trimmed;
        starts_project_closure_command(trimmed)
            || trimmed
                .strip_prefix('#')
                .is_some_and(|body| body.trim_start().starts_with("shellcheck source="))
    }) {
        tags.push(FileContextTag::ProjectClosure);
    }

    if lines.iter().any(|line| {
        line.trimmed
            .strip_prefix('#')
            .is_some_and(|body| matches_directive(body.trim_start()))
    }) {
        tags.push(FileContextTag::DirectiveHandling);
    }

    FileContext::new(tags)
}

#[derive(Debug, Clone)]
struct LineInfo<'a> {
    trimmed: &'a str,
}

fn collect_lines(source: &str) -> Vec<LineInfo<'_>> {
    let mut lines = Vec::new();

    for raw_line in source.split_inclusive('\n') {
        let text = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let trimmed_start = text.trim_start_matches([' ', '\t']);
        let trimmed = trimmed_start.trim_end_matches([' ', '\t']);

        lines.push(LineInfo { trimmed });
    }

    if source.is_empty() {
        lines.push(LineInfo { trimmed: "" });
    }

    lines
}

fn matches_directive(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.starts_with("shellcheck ") || lower == "shellcheck" || lower.starts_with("shuck:")
}

fn starts_project_closure_command(trimmed: &str) -> bool {
    trimmed.starts_with("source ")
        || trimmed.starts_with(". ")
        || trimmed.starts_with("\\source ")
        || trimmed.starts_with("\\. ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{FileContextTag, classify_file_context};
    use crate::ShellDialect;

    #[test]
    fn classifies_project_and_directive_contexts() {
        let source = "\
# shellcheck source=./lib.sh
# shuck: disable=SH-001
helper() { :; }
source ./lib.sh
";
        let context = classify_file_context(
            source,
            Some(Path::new("/tmp/project/tests/libexec/sample_test.func")),
            ShellDialect::Bash,
        );

        assert!(context.has_tag(FileContextTag::ProjectClosure));
        assert!(context.has_tag(FileContextTag::DirectiveHandling));
    }

    #[test]
    fn classifies_escaped_source_commands_as_project_closure() {
        let source = "\
\\. ./lib.sh
helper() { :; }
";
        let context = classify_file_context(
            source,
            Some(Path::new("/tmp/project/tests/helper_swap_test.sh")),
            ShellDialect::Bash,
        );

        assert!(context.has_tag(FileContextTag::ProjectClosure));
    }

    #[test]
    fn ordinary_shell_files_stay_untagged() {
        let context = classify_file_context(
            "#!/bin/sh\necho ok\n",
            Some(Path::new("/tmp/project/main.sh")),
            ShellDialect::Sh,
        );

        assert!(context.tags().is_empty());
    }
}
