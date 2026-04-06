use std::path::Path;

use shuck_ast::{Position, Span};

use crate::ShellDialect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileContextTag {
    ShellSpec,
    GeneratedConfigure,
    HelperLibrary,
    TestHarness,
    ProjectClosure,
    DirectiveHandling,
}

impl FileContextTag {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ShellSpec => "shellspec",
            Self::GeneratedConfigure => "generated-configure",
            Self::HelperLibrary => "helper-library",
            Self::TestHarness => "test-harness",
            Self::ProjectClosure => "project-closure",
            Self::DirectiveHandling => "directive-handling",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextRegionKind {
    ShellSpecParametersBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextRegion {
    pub kind: ContextRegionKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileContext {
    tags: Vec<FileContextTag>,
    regions: Vec<ContextRegion>,
}

impl FileContext {
    pub fn new(tags: Vec<FileContextTag>, mut regions: Vec<ContextRegion>) -> Self {
        let mut tags = tags;
        tags.sort_unstable();
        tags.dedup();
        regions.sort_unstable_by_key(|region| (region.span.start.offset, region.span.end.offset));

        Self { tags, regions }
    }

    pub fn tags(&self) -> &[FileContextTag] {
        &self.tags
    }

    pub fn regions(&self) -> &[ContextRegion] {
        &self.regions
    }

    pub fn has_tag(&self, tag: FileContextTag) -> bool {
        self.tags.contains(&tag)
    }

    pub fn span_intersects_kind(&self, kind: ContextRegionKind, span: Span) -> bool {
        self.regions.iter().any(|region| {
            region.kind == kind
                && region.span.start.offset < span.end.offset
                && span.start.offset < region.span.end.offset
        })
    }
}

pub fn classify_file_context(
    source: &str,
    path: Option<&Path>,
    _shell: ShellDialect,
) -> FileContext {
    let path_lower = path.map(|path| path.to_string_lossy().to_ascii_lowercase());
    let path_tokens = path
        .map(path_tokens)
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    let lines = collect_lines(source);

    let has_shellspec_path = path_lower
        .as_deref()
        .is_some_and(|value| value.contains("shellspec"))
        || path
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with("_spec.sh"));
    let has_shellspec_dsl = lines.iter().any(|line| {
        line.indent == 0
            && !line.trimmed.starts_with('#')
            && shellspec_header_kind(line.trimmed).is_some()
    });

    let mut tags = Vec::new();
    if has_shellspec_path && has_shellspec_dsl {
        tags.push(FileContextTag::ShellSpec);
    }

    if path
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("configure"))
        && autoconf_markers_present(source)
    {
        tags.push(FileContextTag::GeneratedConfigure);
    }

    if path_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "lib" | "libexec" | "completion" | "plugins" | "helpers"
        )
    }) || path_lower.as_deref().is_some_and(|value| {
        value.contains("completions-core") || value.contains("completions-fallback")
    }) || path
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".func"))
    {
        if source_defines_function(&lines) {
            tags.push(FileContextTag::HelperLibrary);
        }
    }

    if path_tokens
        .iter()
        .any(|token| matches!(token.as_str(), "test" | "tests" | "spec"))
        || path
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let lower = name.to_ascii_lowercase();
                lower.contains("_test.") || lower.contains("_spec.")
            })
    {
        tags.push(FileContextTag::TestHarness);
    }

    if lines.iter().any(|line| {
        let trimmed = line.trimmed;
        trimmed.starts_with("source ")
            || trimmed.starts_with(". ")
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

    let regions = if tags.contains(&FileContextTag::ShellSpec) {
        shellspec_parameter_regions(&lines)
    } else {
        Vec::new()
    };

    FileContext::new(tags, regions)
}

#[derive(Debug, Clone)]
struct LineInfo<'a> {
    indent: usize,
    text: &'a str,
    trimmed: &'a str,
    span: Span,
}

fn collect_lines(source: &str) -> Vec<LineInfo<'_>> {
    let mut lines = Vec::new();
    let mut line_number = 1usize;
    let mut offset = 0usize;

    for raw_line in source.split_inclusive('\n') {
        let text = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let trimmed_start = text.trim_start_matches([' ', '\t']);
        let trimmed = trimmed_start.trim_end_matches([' ', '\t']);
        let indent = text.len() - trimmed_start.len();
        let start = Position {
            line: line_number,
            column: 1,
            offset,
        };
        let end = start.advanced_by(text);

        lines.push(LineInfo {
            indent,
            text,
            trimmed,
            span: Span::from_positions(start, end),
        });

        line_number += 1;
        offset += raw_line.len();
    }

    if source.is_empty() {
        lines.push(LineInfo {
            indent: 0,
            text: "",
            trimmed: "",
            span: Span::new(),
        });
    }

    lines
}

fn path_tokens(path: &Path) -> Vec<String> {
    path.iter()
        .filter_map(|part| part.to_str())
        .flat_map(|part| {
            part.split(|char: char| !char.is_ascii_alphanumeric())
                .filter(|token| !token.is_empty())
                .map(|token| token.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn matches_directive(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.starts_with("shellcheck ") || lower == "shellcheck" || lower.starts_with("shuck:")
}

fn autoconf_markers_present(source: &str) -> bool {
    source.contains("Generated by GNU Autoconf")
        || source.contains("as_lineno")
        || source.contains("ac_cv_")
        || source.contains("ac_cs_")
}

fn source_defines_function(lines: &[LineInfo<'_>]) -> bool {
    lines
        .iter()
        .any(|line| probable_function_definition(line.trimmed))
}

fn probable_function_definition(trimmed: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        return rest.contains('{');
    }

    trimmed.contains("() {") || trimmed.contains("(){")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellSpecHeaderKind {
    Parameters,
    Other,
}

fn shellspec_header_kind(trimmed: &str) -> Option<ShellSpecHeaderKind> {
    let leading = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|char| matches!(char, '"' | '\''));

    match leading {
        "Describe" | "Context" | "It" | "Specify" | "When" | "Mock" => {
            Some(ShellSpecHeaderKind::Other)
        }
        "Parameters" => Some(ShellSpecHeaderKind::Parameters),
        _ if leading.starts_with("Before") || leading.starts_with("After") => {
            Some(ShellSpecHeaderKind::Other)
        }
        _ => None,
    }
}

fn shellspec_parameter_regions(lines: &[LineInfo<'_>]) -> Vec<ContextRegion> {
    let mut regions = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = &lines[index];
        let Some(ShellSpecHeaderKind::Parameters) = (line.indent == 0)
            .then(|| shellspec_header_kind(line.trimmed))
            .flatten()
        else {
            index += 1;
            continue;
        };

        let mut span = line.span;
        let mut next = index + 1;

        while next < lines.len() {
            let line = &lines[next];

            if line.trimmed.is_empty() {
                next += 1;
                continue;
            }

            if line.indent == 0 {
                if line.trimmed == "End" {
                    span = span.merge(line.span);
                }
                break;
            }

            if line.indent > 0 || quoted_parameter_line(line.text) {
                span = span.merge(line.span);
                next += 1;
                continue;
            }

            break;
        }

        regions.push(ContextRegion {
            kind: ContextRegionKind::ShellSpecParametersBlock,
            span,
        });
        index = next;
    }

    regions
}

fn quoted_parameter_line(text: &str) -> bool {
    let trimmed = text.trim_start_matches([' ', '\t']);
    trimmed.starts_with('"') || trimmed.starts_with('\'')
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        ContextRegionKind, FileContextTag, classify_file_context, shellspec_parameter_regions,
    };
    use crate::ShellDialect;

    #[test]
    fn classifies_shellspec_files_and_parameter_regions() {
        let source = "\
Describe 'clone'
Parameters
  \"test\"
  \"test$SHELLSPEC_LF\"
End
It 'still shell'
";
        let context = classify_file_context(
            source,
            Some(Path::new(
                "/tmp/ko1nksm__shellspec__spec__core__clone_spec.sh",
            )),
            ShellDialect::Sh,
        );

        assert!(context.has_tag(FileContextTag::ShellSpec));
        assert!(context.has_tag(FileContextTag::TestHarness));
        assert_eq!(context.regions().len(), 1);
        assert_eq!(
            context.regions()[0].kind,
            ContextRegionKind::ShellSpecParametersBlock
        );
        assert_eq!(
            context.regions()[0].span.slice(source),
            "Parameters\n  \"test\"\n  \"test$SHELLSPEC_LF\"\nEnd"
        );
    }

    #[test]
    fn classifies_generated_configure_files() {
        let source = "\
# Generated by GNU Autoconf 2.71
as_lineno=${as_lineno-$LINENO}
ac_cv_env_CC_value=${CC}
";
        let context = classify_file_context(
            source,
            Some(Path::new("/tmp/examples/native/configure")),
            ShellDialect::Sh,
        );

        assert!(context.has_tag(FileContextTag::GeneratedConfigure));
    }

    #[test]
    fn classifies_helper_test_project_and_directive_contexts() {
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

        assert!(context.has_tag(FileContextTag::HelperLibrary));
        assert!(context.has_tag(FileContextTag::TestHarness));
        assert!(context.has_tag(FileContextTag::ProjectClosure));
        assert!(context.has_tag(FileContextTag::DirectiveHandling));
    }

    #[test]
    fn ordinary_shell_files_stay_untagged() {
        let context = classify_file_context(
            "#!/bin/sh\necho ok\n",
            Some(Path::new("/tmp/project/main.sh")),
            ShellDialect::Sh,
        );

        assert!(context.tags().is_empty());
        assert!(context.regions().is_empty());
    }

    #[test]
    fn shellspec_parameter_regions_stop_at_next_top_level_header() {
        let lines = super::collect_lines(
            "\
Parameters
  \"a\"
Describe 'next'
",
        );
        let regions = shellspec_parameter_regions(&lines);

        assert_eq!(regions.len(), 1);
        assert_eq!(
            regions[0]
                .span
                .slice("Parameters\n  \"a\"\nDescribe 'next'\n"),
            "Parameters\n  \"a\""
        );
    }
}
