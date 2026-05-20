use anyhow::Error;
use lsp_types as types;
use shuck_ast::{Command, CompoundCommand, Span, Stmt, StmtSeq, TextRange};
use shuck_formatter::FormattedSource;

use crate::edit::RangeExt;
use crate::session::{Client, DocumentSnapshot};

pub(crate) type FormatResponse = Option<Vec<types::TextEdit>>;

pub(crate) fn format_document(
    snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentFormattingParams,
) -> crate::server::Result<FormatResponse> {
    let query = snapshot.query();
    let source = query.document().contents();
    let formatted = shuck_formatter::format_source(
        source,
        query.file_path().as_deref(),
        snapshot.shuck_settings().formatter(),
    )
    .map_err(Error::new)?;

    Ok(Some(match formatted {
        FormattedSource::Unchanged => Vec::new(),
        FormattedSource::Formatted(code) => crate::edit::single_replacement_edit(
            source,
            &code,
            query.document().index(),
            snapshot.encoding(),
        )
        .into_iter()
        .collect(),
    }))
}

pub(crate) fn format_range(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::DocumentRangeFormattingParams,
) -> crate::server::Result<FormatResponse> {
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let requested = params
        .range
        .to_text_range(source, analysis.line_index(), snapshot.encoding());
    let Some(statement_span) =
        smallest_statement_span_containing(&analysis.parse_result().file.body, requested)
    else {
        return Ok(None);
    };
    let statement_range = statement_span.to_range();
    let statement_source =
        &source[usize::from(statement_range.start())..usize::from(statement_range.end())];
    let formatted = shuck_formatter::format_source(
        statement_source,
        snapshot.query().file_path().as_deref(),
        snapshot.shuck_settings().formatter(),
    )
    .map_err(Error::new)?;

    Ok(Some(format_response_for_range(
        source,
        statement_range,
        formatted,
        analysis.line_index(),
        snapshot.encoding(),
    )))
}

fn format_response_for_range(
    source: &str,
    range: TextRange,
    formatted: FormattedSource,
    line_index: &shuck_indexer::LineIndex,
    encoding: crate::PositionEncoding,
) -> Vec<types::TextEdit> {
    match formatted {
        FormattedSource::Unchanged => Vec::new(),
        FormattedSource::Formatted(code) => crate::edit::single_replacement_edit_in_range(
            source, range, &code, line_index, encoding,
        )
        .into_iter()
        .collect(),
    }
}

fn smallest_statement_span_containing(body: &StmtSeq, range: TextRange) -> Option<Span> {
    let mut best = None;
    collect_statement_span(body, range, &mut best);
    best
}

fn collect_statement_span(body: &StmtSeq, range: TextRange, best: &mut Option<Span>) {
    for stmt in body.as_slice() {
        if !span_contains_text_range(stmt.span, range) {
            continue;
        }
        record_smaller_span(best, stmt.span);
        collect_nested_statement_spans(stmt, range, best);
    }
}

fn collect_nested_statement_spans(stmt: &Stmt, range: TextRange, best: &mut Option<Span>) {
    match &stmt.command {
        Command::Binary(command) => {
            collect_nested_statement_spans(&command.left, range, best);
            collect_nested_statement_spans(&command.right, range, best);
        }
        Command::Compound(command) => collect_compound_statement_spans(command, range, best),
        Command::Function(command) => collect_nested_statement_spans(&command.body, range, best),
        Command::AnonymousFunction(command) => {
            collect_nested_statement_spans(&command.body, range, best);
        }
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_compound_statement_spans(
    command: &CompoundCommand,
    range: TextRange,
    best: &mut Option<Span>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_statement_span(&command.condition, range, best);
            collect_statement_span(&command.then_branch, range, best);
            for (condition, body) in &command.elif_branches {
                collect_statement_span(condition, range, best);
                collect_statement_span(body, range, best);
            }
            if let Some(body) = &command.else_branch {
                collect_statement_span(body, range, best);
            }
        }
        CompoundCommand::For(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::Repeat(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::Foreach(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::ArithmeticFor(command) => {
            collect_statement_span(&command.body, range, best)
        }
        CompoundCommand::While(command) => {
            collect_statement_span(&command.condition, range, best);
            collect_statement_span(&command.body, range, best);
        }
        CompoundCommand::Until(command) => {
            collect_statement_span(&command.condition, range, best);
            collect_statement_span(&command.body, range, best);
        }
        CompoundCommand::Case(command) => {
            for item in &command.cases {
                collect_statement_span(&item.body, range, best);
            }
        }
        CompoundCommand::Select(command) => collect_statement_span(&command.body, range, best),
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            collect_statement_span(body, range, best);
        }
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_nested_statement_spans(command, range, best);
            }
        }
        CompoundCommand::Coproc(command) => {
            collect_nested_statement_spans(&command.body, range, best);
        }
        CompoundCommand::Always(command) => {
            collect_statement_span(&command.body, range, best);
            collect_statement_span(&command.always_body, range, best);
        }
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
    }
}

fn record_smaller_span(best: &mut Option<Span>, candidate: Span) {
    if best.is_none_or(|current| span_width(candidate) < span_width(current)) {
        *best = Some(candidate);
    }
}

fn span_width(span: Span) -> usize {
    span.end.offset.saturating_sub(span.start.offset)
}

fn span_contains_text_range(span: Span, range: TextRange) -> bool {
    span.start.offset <= usize::from(range.start()) && usize::from(range.end()) <= span.end.offset
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{ClientCapabilities, PositionEncodingKind, Url};

    use super::*;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

    #[test]
    fn document_formatting_uses_shuck_formatter() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-format-document-tests");
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
            TextDocument::new("if true; then\necho ok\nfi\n".to_owned(), 1)
                .with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_document(
            snapshot,
            &client,
            types::DocumentFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("document formatting should succeed")
        .expect("document formatting should return an edit list");

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, types::Position::new(1, 0));
        assert_eq!(edits[0].range.end, types::Position::new(1, 0));
        assert_eq!(edits[0].new_text, "\t");
    }

    #[test]
    fn range_formatting_returns_empty_edits_for_already_formatted_buffer() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-format-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities {
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![PositionEncodingKind::UTF16]),
                    ..Default::default()
                }),
                ..Default::default()
            },
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
            TextDocument::new("echo hi\n".to_owned(), 1).with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_range(
            snapshot,
            &client,
            types::DocumentRangeFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                range: types::Range::new(types::Position::new(0, 0), types::Position::new(0, 7)),
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("range formatting should succeed")
        .expect("range formatting should return an edit list");

        assert!(edits.is_empty());
    }

    #[test]
    fn range_formatting_returns_none_for_ranges_without_one_complete_statement() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-format-tests-partial");
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
            TextDocument::new("echo one\necho two\n".to_owned(), 1).with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_range(
            snapshot,
            &client,
            types::DocumentRangeFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                range: types::Range::new(types::Position::new(0, 2), types::Position::new(1, 2)),
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("range formatting should succeed");

        assert!(edits.is_none());
    }

    #[test]
    fn range_formatting_edit_helper_does_not_escape_statement_range() {
        let source = "echo one\necho two\n";
        let index = shuck_indexer::LineIndex::new(source);
        let edits = format_response_for_range(
            source,
            TextRange::new(shuck_ast::TextSize::new(9), shuck_ast::TextSize::new(18)),
            FormattedSource::Formatted("printf two\n".to_owned()),
            &index,
            PositionEncoding::UTF16,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, types::Position::new(1, 0));
        assert_eq!(edits[0].range.end, types::Position::new(1, 4));
        assert_eq!(edits[0].new_text, "printf");
    }
}
