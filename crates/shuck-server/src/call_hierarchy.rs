//! Cross-file call hierarchy (spec 025).
//!
//! `incomingCalls` / `outgoingCalls` answer across the whole workspace by
//! building a [`WorkspaceCallIndex`] per request: every workspace shell file
//! (open buffer preferred over disk) is parsed and projected into call facts,
//! its determinable source edges resolved, and the two directions answered as
//! traversals of the resulting call graph. `prepareCallHierarchy` stays a
//! single-file identity step; the item it returns (name + file URI) is enough
//! for the index queries to locate the node.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lsp_types as types;
use shuck_ast::{Name, Span};
use shuck_indexer::LineIndex;
use shuck_linter::ShellDialect;
use shuck_semantic::{
    CallNodeKind, CrossFileCall, FileCallFacts, WorkspaceCallIndex, resolve_source_ref_targets,
};

use crate::PositionEncoding;
use crate::editor::analyze_editor_document;
use crate::symbols::WorkspaceOpenDocument;

pub(crate) type IncomingResponse = Option<Vec<types::CallHierarchyIncomingCall>>;
pub(crate) type OutgoingResponse = Option<Vec<types::CallHierarchyOutgoingCall>>;

/// Workspace state needed to build the call index for one request.
pub(crate) struct CallHierarchyContext {
    pub(crate) workspace_roots: Vec<PathBuf>,
    pub(crate) open_documents: Vec<WorkspaceOpenDocument>,
    pub(crate) encoding: PositionEncoding,
}

/// Source text and line index for one indexed file, retained for span→range
/// mapping of cross-file results.
struct FileText {
    source: String,
    line_index: LineIndex,
}

/// The per-request index plus the text needed to render results.
struct BuiltIndex {
    index: WorkspaceCallIndex,
    texts: BTreeMap<PathBuf, FileText>,
    encoding: PositionEncoding,
}

pub(crate) fn incoming_calls(
    context: CallHierarchyContext,
    params: types::CallHierarchyIncomingCallsParams,
) -> crate::server::Result<IncomingResponse> {
    let Some((path, name)) = item_identity(&params.item) else {
        return Ok(None);
    };
    let built = BuiltIndex::build(&context);
    let calls = built
        .index
        .incoming(&path, &name)
        .into_iter()
        .filter_map(|call| {
            let from = built.item_for(&call)?;
            Some(types::CallHierarchyIncomingCall {
                from_ranges: built.ranges_in(&call.path, &call.call_spans),
                from,
            })
        })
        .collect();
    Ok(Some(calls))
}

pub(crate) fn outgoing_calls(
    context: CallHierarchyContext,
    params: types::CallHierarchyOutgoingCallsParams,
) -> crate::server::Result<OutgoingResponse> {
    let Some((path, name)) = item_identity(&params.item) else {
        return Ok(None);
    };
    let built = BuiltIndex::build(&context);
    // Outgoing call-token spans live in the queried file.
    let calls = built
        .index
        .outgoing(&path, &CallNodeKind::Function(name))
        .into_iter()
        .filter_map(|call| {
            let to = built.item_for(&call)?;
            Some(types::CallHierarchyOutgoingCall {
                from_ranges: built.ranges_in(&path, &call.call_spans),
                to,
            })
        })
        .collect();
    Ok(Some(calls))
}

/// The queried node's identity: its file path and function name. The single-file
/// `prepare` step already stamped the item with the function name and its file
/// URI, so no `data` round-trip is required.
fn item_identity(item: &types::CallHierarchyItem) -> Option<(PathBuf, Name)> {
    let path = canonical(&item.uri.to_file_path().ok()?);
    Some((path, Name::from(item.name.as_str())))
}

impl BuiltIndex {
    fn build(context: &CallHierarchyContext) -> Self {
        let mut index = WorkspaceCallIndex::new();
        let mut texts: BTreeMap<PathBuf, FileText> = BTreeMap::new();

        let mut open_paths = Vec::new();
        for open in &context.open_documents {
            let Some(path) = open.uri.to_file_path().ok().map(|path| canonical(&path)) else {
                continue;
            };
            open_paths.push(path.clone());
            insert_file(&mut index, &mut texts, &path, open.document.contents());
        }

        for file in discover_closed_shell_files(&context.workspace_roots, &open_paths) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            insert_file(&mut index, &mut texts, &file, &source);
        }

        Self {
            index,
            texts,
            encoding: context.encoding,
        }
    }

    /// Builds an LSP item for one end of a cross-file edge. Functions carry their
    /// file URI and definition ranges; a top-level caller becomes a MODULE node.
    fn item_for(&self, call: &CrossFileCall) -> Option<types::CallHierarchyItem> {
        let uri = types::Url::from_file_path(&call.path).ok()?;
        match &call.node {
            CallNodeKind::Function(name) => {
                let text = self.texts.get(&call.path)?;
                let range = self.range_of(&call.path, call.def_span?)?;
                let selection_range = call
                    .selection_span
                    .and_then(|span| self.range_of(&call.path, span))
                    .unwrap_or(range);
                let _ = text;
                Some(types::CallHierarchyItem {
                    name: name.to_string(),
                    kind: types::SymbolKind::FUNCTION,
                    tags: None,
                    detail: None,
                    uri,
                    range,
                    selection_range,
                    data: None,
                })
            }
            CallNodeKind::TopLevel => {
                let start = types::Position::new(0, 0);
                let range = types::Range { start, end: start };
                Some(types::CallHierarchyItem {
                    name: top_level_label(&call.path),
                    kind: types::SymbolKind::MODULE,
                    tags: None,
                    detail: Some("script top level".to_owned()),
                    uri,
                    range,
                    selection_range: range,
                    data: None,
                })
            }
        }
    }

    fn ranges_in(&self, path: &Path, spans: &[Span]) -> Vec<types::Range> {
        spans
            .iter()
            .filter_map(|span| self.range_of(path, *span))
            .collect()
    }

    fn range_of(&self, path: &Path, span: Span) -> Option<types::Range> {
        let text = self.texts.get(path)?;
        Some(crate::edit::to_lsp_range(
            span.to_range(),
            &text.source,
            &text.line_index,
            self.encoding,
        ))
    }
}

fn insert_file(
    index: &mut WorkspaceCallIndex,
    texts: &mut BTreeMap<PathBuf, FileText>,
    path: &Path,
    source: &str,
) {
    let key = canonical(path);
    let shell = ShellDialect::infer(source, Some(path));
    let model = analyze_editor_document(source, Some(path), shell);
    // Base-directory resolution covers relative hints and literal includes;
    // configured `source-paths` roots are a follow-up.
    let edges = model
        .source_refs()
        .iter()
        .flat_map(|source_ref| resolve_source_ref_targets(path, source_ref, &[], Path::new("")))
        .map(|target| canonical(&target))
        .collect::<Vec<_>>();
    index.insert(key.clone(), FileCallFacts::project(&model, edges));
    texts.insert(
        key,
        FileText {
            source: source.to_owned(),
            line_index: LineIndex::new(source),
        },
    );
}

fn discover_closed_shell_files(roots: &[PathBuf], open_paths: &[PathBuf]) -> Vec<PathBuf> {
    use shuck_discover::{DiscoveryOptions, FileKind, discover_files};

    let mut files: BTreeMap<PathBuf, ()> = BTreeMap::new();
    for root in roots {
        let discovered = match discover_files(
            std::slice::from_ref(root),
            root,
            &DiscoveryOptions {
                respect_gitignore: true,
                parallel: true,
                use_config_roots: true,
                ..DiscoveryOptions::default()
            },
        ) {
            Ok(files) => files,
            Err(error) => {
                tracing::warn!(
                    "call hierarchy: failed to discover files in {}: {error}",
                    root.display()
                );
                continue;
            }
        };
        for file in discovered {
            if file.kind != FileKind::Shell {
                continue;
            }
            let path = canonical(&file.absolute_path);
            if open_paths.contains(&path) {
                continue;
            }
            files.entry(path).or_default();
        }
    }
    files.into_keys().collect()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn top_level_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "script".to_owned())
}
