use lsp_types as types;
use shuck_linter::{
    ShellCheckCodeMap, SuppressionAction, SuppressionSource, rule_metadata_by_code,
};
use shuck_semantic::{BindingAttributes, EditorHover, EditorSymbolKind, ScopeKind, SemanticModel};

use crate::edit::RangeExt;
use crate::session::{Client, DocumentSnapshot};

pub(crate) fn hover(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::HoverParams,
) -> crate::server::Result<Option<types::Hover>> {
    let Some(shell) = crate::lint::document_shell(&snapshot) else {
        return Ok(None);
    };

    let query = snapshot.query();
    let source = query.document().contents();
    let path = query.file_path();
    let parsed = crate::editor::parse_editor_document(source, shell);
    let shellcheck_map = ShellCheckCodeMap::default();
    let position = params.text_document_position_params.position;
    let offset = usize::from(
        types::Range {
            start: position,
            end: position,
        }
        .to_text_range(source, query.document().index(), snapshot.encoding())
        .start(),
    );

    if let Some(hover) = directive_hover(
        &snapshot,
        source,
        query.document().index(),
        parsed.indexer.comment_index(),
        &shellcheck_map,
        params.text_document_position_params.position,
        offset,
    ) {
        return Ok(Some(hover));
    }

    let semantic = crate::editor::semantic_for_parsed_document(&parsed, source, path.as_deref());
    let Some(semantic_hover) = semantic.editor_query().hover_at_offset(offset) else {
        return Ok(None);
    };
    Ok(Some(render_semantic_hover(
        &snapshot,
        source,
        query.document().index(),
        &semantic,
        &semantic_hover,
    )))
}

fn directive_hover(
    snapshot: &DocumentSnapshot,
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    comment_index: &shuck_indexer::CommentIndex,
    shellcheck_map: &ShellCheckCodeMap,
    position: types::Position,
    offset: usize,
) -> Option<types::Hover> {
    let directives = shuck_linter::parse_directives(source, comment_index, shellcheck_map);
    let line = usize::try_from(position.line).unwrap_or_default() + 1;
    let directive = directives.iter().find(|directive| {
        usize::try_from(directive.line).ok() == Some(line)
            && offset >= usize::from(directive.range.start())
            && offset <= usize::from(directive.range.end())
            && matches!(
                (directive.source, directive.action),
                (
                    SuppressionSource::Shuck,
                    SuppressionAction::Ignore
                        | SuppressionAction::Disable
                        | SuppressionAction::DisableFile
                ) | (SuppressionSource::ShellCheck, SuppressionAction::Disable)
            )
    })?;

    let (display_code, canonical_code, start_offset, end_offset) = code_at_offset(
        directive.range.slice(source),
        usize::from(directive.range.start()),
        offset,
    )?;
    let metadata = rule_metadata_by_code(&canonical_code)?;

    let rule_name = humanize_rule_name(&canonical_code);
    let fix_marker = if metadata.fix_description.is_some() {
        "Fix available"
    } else {
        "No auto-fix"
    };
    let mut markdown = format!(
        "# {} ({})\n\n{}\n\n{}\n\n{}",
        rule_name, display_code, metadata.description, fix_marker, metadata.rationale
    );
    if display_code != canonical_code {
        markdown.push_str(&format!("\n\nSee also: {}", canonical_code));
    } else if let Some(rule) = shuck_linter::code_to_rule(&canonical_code)
        && let Some(shellcheck_code) = shellcheck_map.code_for_rule(rule)
    {
        markdown.push_str(&format!("\n\nSee also: SC{shellcheck_code:04}"));
    }

    Some(types::Hover {
        contents: types::HoverContents::Markup(types::MarkupContent {
            kind: types::MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(crate::edit::to_lsp_range(
            shuck_ast::TextRange::new(
                shuck_ast::TextSize::new(start_offset as u32),
                shuck_ast::TextSize::new(end_offset as u32),
            ),
            source,
            line_index,
            snapshot.encoding(),
        )),
    })
}

fn render_semantic_hover(
    snapshot: &DocumentSnapshot,
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    semantic: &SemanticModel,
    hover: &EditorHover,
) -> types::Hover {
    let mut markdown = format!(
        "### {}\n\n{}",
        markdown_code(hover.symbol.name.as_str()),
        symbol_kind_label(hover.symbol.kind)
    );

    if hover.runtime {
        markdown.push_str("\n\nProvided by the active shell runtime.");
    } else {
        markdown.push_str(&format!(
            "\n\nDefined at line {}, column {}.",
            hover.symbol.definition_span.start.line, hover.symbol.definition_span.start.column
        ));
    }

    markdown.push_str(&format!(
        "\n\nScope: {}.",
        scope_summary(semantic, hover.symbol.scope)
    ));

    let attributes = attribute_labels(hover.attributes);
    if !attributes.is_empty() {
        markdown.push_str(&format!("\n\nAttributes: {}.", attributes.join(", ")));
    }
    if hover.imported {
        markdown.push_str("\n\nImported into this analysis.");
    }
    if let Some(count) = hover.function_call_count {
        let noun = if count == 1 { "site" } else { "sites" };
        markdown.push_str(&format!("\n\nFile-local call sites: {count} {noun}."));
    }

    types::Hover {
        contents: types::HoverContents::Markup(types::MarkupContent {
            kind: types::MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(crate::edit::to_lsp_range(
            hover.target_span.to_range(),
            source,
            line_index,
            snapshot.encoding(),
        )),
    }
}

fn markdown_code(text: &str) -> String {
    format!("`{}`", text.replace('`', "\\`"))
}

fn symbol_kind_label(kind: EditorSymbolKind) -> &'static str {
    match kind {
        EditorSymbolKind::Function => "Function",
        EditorSymbolKind::Variable => "Variable",
        EditorSymbolKind::Array => "Array variable",
        EditorSymbolKind::AssociativeArray => "Associative array variable",
        EditorSymbolKind::Declaration => "Declaration",
        EditorSymbolKind::RuntimeName => "Runtime name",
    }
}

fn scope_summary(semantic: &SemanticModel, scope: shuck_semantic::ScopeId) -> String {
    match semantic.scope_kind(scope) {
        ScopeKind::File => "top-level".to_owned(),
        ScopeKind::Function(function) => function
            .static_names()
            .first()
            .map(|name| format!("function {}", markdown_code(name.as_str())))
            .unwrap_or_else(|| "function-local".to_owned()),
        ScopeKind::Subshell => "subshell".to_owned(),
        ScopeKind::CommandSubstitution => "command substitution".to_owned(),
        ScopeKind::Pipeline => "pipeline".to_owned(),
    }
}

fn attribute_labels(attributes: BindingAttributes) -> Vec<&'static str> {
    [
        (BindingAttributes::EXPORTED, "exported"),
        (BindingAttributes::READONLY, "readonly"),
        (BindingAttributes::LOCAL, "local"),
        (BindingAttributes::INTEGER, "integer"),
        (BindingAttributes::ARRAY, "array"),
        (BindingAttributes::ASSOC, "associative array"),
        (BindingAttributes::NAMEREF, "nameref"),
        (BindingAttributes::LOWERCASE, "lowercase"),
        (BindingAttributes::UPPERCASE, "uppercase"),
        (
            BindingAttributes::DECLARATION_INITIALIZED,
            "initialized by declaration",
        ),
    ]
    .into_iter()
    .filter_map(|(flag, label)| attributes.contains(flag).then_some(label))
    .collect()
}

fn code_at_offset(
    text: &str,
    base_offset: usize,
    offset: usize,
) -> Option<(String, String, usize, usize)> {
    let mut search_from = 0usize;
    for token in text.split(|ch: char| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-')) {
        if token.is_empty() {
            continue;
        }
        let start = text[search_from..].find(token)? + search_from;
        search_from = start + token.len();
        let start_offset = base_offset + start;
        let end_offset = start_offset + token.len();
        if offset < start_offset || offset > end_offset {
            continue;
        }

        if let Some(rule) = shuck_linter::code_to_rule(token) {
            return Some((
                token.to_owned(),
                rule.code().to_owned(),
                start_offset,
                end_offset,
            ));
        }
        if let Some(rule) = ShellCheckCodeMap::default().resolve(token) {
            return Some((
                token.to_owned(),
                rule.code().to_owned(),
                start_offset,
                end_offset,
            ));
        }
    }

    None
}

fn humanize_rule_name(code: &str) -> String {
    let Some(rule) = shuck_linter::code_to_rule(code) else {
        return code.to_owned();
    };
    let raw = format!("{rule:?}");
    let mut output = String::new();
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && ch.is_uppercase() {
            output.push(' ');
        }
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, HoverParams, Position, TextDocumentIdentifier,
        TextDocumentPositionParams, Url, WorkDoneProgressParams,
    };
    use shuck_ast::{TextRange, TextSize};
    use shuck_indexer::LineIndex;

    use super::*;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

    fn make_snapshot(source: &str) -> (DocumentSnapshot, Client) {
        make_snapshot_with_encoding(source, PositionEncoding::UTF16)
    }

    fn make_snapshot_with_encoding(
        source: &str,
        encoding: PositionEncoding,
    ) -> (DocumentSnapshot, Client) {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-hover-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            encoding,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = script_uri();
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id("shellscript"),
        );

        (
            session
                .take_snapshot(uri)
                .expect("test document should produce a snapshot"),
            client,
        )
    }

    fn script_uri() -> Url {
        Url::from_file_path(std::env::temp_dir().join("shuck-server-hover-tests/script.sh"))
            .expect("script path should convert to a URL")
    }

    fn hover_params(source: &str, needle: &str, encoding: PositionEncoding) -> HoverParams {
        let offset = source.find(needle).expect("needle should exist") + needle.len() / 2;
        HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: script_uri() },
                position: position_for_offset(source, offset, encoding),
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        }
    }

    fn position_for_offset(source: &str, offset: usize, encoding: PositionEncoding) -> Position {
        let index = LineIndex::new(source);
        crate::edit::to_lsp_range(
            TextRange::new(TextSize::new(offset as u32), TextSize::new(offset as u32)),
            source,
            &index,
            encoding,
        )
        .start
    }

    fn hover_markdown(hover: types::Hover) -> String {
        let types::HoverContents::Markup(markup) = hover.contents else {
            panic!("expected markdown hover content");
        };
        markup.value
    }

    #[test]
    fn hover_resolves_shuck_ignore_codes() {
        let (snapshot, client) = make_snapshot("#!/bin/bash\necho $foo  # shuck: ignore=C006\n");
        let hover = hover(
            snapshot,
            &client,
            HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::from_file_path(
                            std::env::temp_dir()
                                .join("shuck-server-hover-tests")
                                .join("script.sh"),
                        )
                        .expect("script path should convert to a URL"),
                    },
                    position: Position::new(1, 30),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        )
        .expect("hover request should succeed")
        .expect("directive hover should be present");

        let types::HoverContents::Markup(markup) = hover.contents else {
            panic!("expected markdown hover content");
        };
        assert!(markup.value.contains("Undefined Variable"));
        assert!(markup.value.contains("C006"));
        assert!(markup.value.contains("Fix available") || markup.value.contains("No auto-fix"));
    }

    #[test]
    fn hover_resolves_shellcheck_disable_codes() {
        let (snapshot, client) =
            make_snapshot("#!/bin/bash\necho $foo  # shellcheck disable=SC2154\n");
        let hover = hover(
            snapshot,
            &client,
            HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::from_file_path(
                            std::env::temp_dir()
                                .join("shuck-server-hover-tests")
                                .join("script.sh"),
                        )
                        .expect("script path should convert to a URL"),
                    },
                    position: Position::new(1, 37),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        )
        .expect("hover request should succeed")
        .expect("directive hover should be present");

        let types::HoverContents::Markup(markup) = hover.contents else {
            panic!("expected markdown hover content");
        };
        assert!(markup.value.contains("Undefined Variable"));
        assert!(markup.value.contains("SC2154"));
        assert!(markup.value.contains("See also: C006"));
        assert!(markup.value.contains("Fix available") || markup.value.contains("No auto-fix"));
    }

    #[test]
    fn hover_falls_back_to_semantic_symbols() {
        let source = "#!/bin/bash\nname=world\nprintf '%s\\n' \"$name\"\n";
        let (snapshot, client) = make_snapshot(source);
        let hover = hover(
            snapshot,
            &client,
            hover_params(source, "name\"", PositionEncoding::UTF16),
        )
        .expect("hover request should succeed")
        .expect("semantic hover should be present");

        let markdown = hover_markdown(hover.clone());
        assert!(markdown.contains("`name`"));
        assert!(markdown.contains("Variable"));
        assert!(markdown.contains("Defined at line 2, column 1"));
        assert!(markdown.contains("Scope: top-level"));
        let range = hover.range.expect("semantic hover should have a range");
        assert_eq!(range.start.line, 2);
    }

    #[test]
    fn hover_reports_semantic_function_call_details() {
        let source = "#!/bin/bash\nbuild() { :; }\nbuild\n";
        let (snapshot, client) = make_snapshot(source);
        let hover = hover(
            snapshot,
            &client,
            hover_params(source, "build\n", PositionEncoding::UTF16),
        )
        .expect("hover request should succeed")
        .expect("function hover should be present");

        let markdown = hover_markdown(hover);
        assert!(markdown.contains("`build`"));
        assert!(markdown.contains("Function"));
        assert!(markdown.contains("File-local call sites: 1 site"));
    }

    #[test]
    fn hover_reports_runtime_names() {
        let source = "#!/bin/bash\nprintf '%s\\n' \"$HOME\"\n";
        let (snapshot, client) = make_snapshot(source);
        let hover = hover(
            snapshot,
            &client,
            hover_params(source, "HOME", PositionEncoding::UTF16),
        )
        .expect("hover request should succeed")
        .expect("runtime hover should be present");

        let markdown = hover_markdown(hover);
        assert!(markdown.contains("`HOME`"));
        assert!(markdown.contains("Runtime name"));
        assert!(markdown.contains("Provided by the active shell runtime"));
    }

    #[test]
    fn semantic_hover_ranges_use_negotiated_position_encoding() {
        let source = "#!/bin/bash\nname=world\nprintf 'é' \"$name\"\n";
        let (utf16_snapshot, utf16_client) =
            make_snapshot_with_encoding(source, PositionEncoding::UTF16);
        let utf16_hover = hover(
            utf16_snapshot,
            &utf16_client,
            hover_params(source, "name\"", PositionEncoding::UTF16),
        )
        .expect("hover request should succeed")
        .expect("utf16 hover should be present");

        let (utf8_snapshot, utf8_client) =
            make_snapshot_with_encoding(source, PositionEncoding::UTF8);
        let utf8_hover = hover(
            utf8_snapshot,
            &utf8_client,
            hover_params(source, "name\"", PositionEncoding::UTF8),
        )
        .expect("hover request should succeed")
        .expect("utf8 hover should be present");

        let utf16_range = utf16_hover.range.expect("utf16 hover range");
        let utf8_range = utf8_hover.range.expect("utf8 hover range");
        assert_eq!(utf16_range.start.line, utf8_range.start.line);
        assert!(utf8_range.start.character > utf16_range.start.character);
    }

    #[test]
    fn hover_returns_none_for_non_shell_documents() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-hover-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_root.join("README.md"))
            .expect("document path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("# shellcheck disable=SC2154\n".to_owned(), 1)
                .with_language_id("markdown"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let hover = hover(
            snapshot,
            &client,
            HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position::new(0, 22),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        )
        .expect("hover request should succeed");

        assert!(hover.is_none());
    }
}
