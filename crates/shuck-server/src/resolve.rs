use lsp_types as types;
use shuck_linter::{ShellCheckCodeMap, SuppressionAction, SuppressionSource, rule_metadata_by_code};
use shuck_parser::parser::Parser;

use crate::edit::RangeExt;
use crate::session::{Client, DocumentSnapshot};

pub(crate) fn hover(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::HoverParams,
) -> crate::server::Result<Option<types::Hover>> {
    if crate::lint::document_shell(&snapshot).is_none() {
        return Ok(None);
    }

    let query = snapshot.query();
    let source = query.document().contents();
    let parse_result = Parser::new(source).parse();
    let indexer = shuck_indexer::Indexer::new(source, &parse_result);
    let shellcheck_map = ShellCheckCodeMap::default();
    let directives =
        shuck_linter::parse_directives(source, indexer.comment_index(), &shellcheck_map);
    let position = params.text_document_position_params.position;
    let offset = usize::from(
        types::Range {
            start: position,
            end: position,
        }
        .to_text_range(source, query.document().index(), snapshot.encoding())
        .start(),
    );
    let line = usize::try_from(params.text_document_position_params.position.line)
        .unwrap_or_default()
        + 1;
    let Some(directive) = directives.iter().find(|directive| {
        usize::try_from(directive.line).ok() == Some(line)
            && offset >= usize::from(directive.range.start())
            && offset <= usize::from(directive.range.end())
            && matches!(
                (directive.source, directive.action),
                (SuppressionSource::Shuck, SuppressionAction::Ignore | SuppressionAction::Disable | SuppressionAction::DisableFile)
                    | (SuppressionSource::ShellCheck, SuppressionAction::Disable)
            )
    }) else {
        return Ok(None);
    };

    let Some((display_code, canonical_code, start_offset, end_offset)) =
        code_at_offset(directive.range.slice(source), usize::from(directive.range.start()), offset)
    else {
        return Ok(None);
    };
    let Some(metadata) = rule_metadata_by_code(&canonical_code) else {
        return Ok(None);
    };

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

    Ok(Some(types::Hover {
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
            query.document().index(),
            snapshot.encoding(),
        )),
    }))
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
            return Some((token.to_owned(), rule.code().to_owned(), start_offset, end_offset));
        }
        if let Some(rule) = ShellCheckCodeMap::default().resolve(token) {
            return Some((token.to_owned(), rule.code().to_owned(), start_offset, end_offset));
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

    use super::*;
    use crate::{Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces};

    fn make_snapshot(source: &str) -> (DocumentSnapshot, Client) {
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

        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to a URL");
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
