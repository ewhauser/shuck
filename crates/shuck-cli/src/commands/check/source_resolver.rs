//! Native resolver for `assume-source` / `follow-source` hint paths.
//!
//! The source closure already resolves a directive path relative to the
//! annotating file's own directory. This resolver adds the configured
//! `[lint] source-paths` roots as additional search directories, mirroring the
//! ShellCheck-compat `--source-path` behavior, and exposes a helper for
//! resolving a `SourceRef` to concrete on-disk targets (used to decide which
//! `follow-source` files to lint).

use std::path::{Path, PathBuf};

use shuck_semantic::{SourcePathResolver, SourceRef, SourceRefKind};

/// Resolves relative source-hint paths against configured roots.
#[derive(Debug, Clone)]
pub(super) struct NativeSourceResolver {
    cwd: PathBuf,
    source_paths: Vec<String>,
}

impl NativeSourceResolver {
    pub(super) fn new(cwd: PathBuf, source_paths: Vec<String>) -> Self {
        Self { cwd, source_paths }
    }

    /// Whether any extra roots are configured. When empty, the closure's own
    /// base-directory resolution is sufficient and this resolver adds nothing.
    pub(super) fn has_roots(&self) -> bool {
        !self.source_paths.is_empty()
    }
}

impl SourcePathResolver for NativeSourceResolver {
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        let mut resolved = Vec::new();
        for root in &self.source_paths {
            let root_path = if root == "SCRIPTDIR" {
                source_path.parent().unwrap_or(Path::new("")).to_path_buf()
            } else {
                let root_path = PathBuf::from(root);
                if root_path.is_absolute() {
                    root_path
                } else {
                    self.cwd.join(root_path)
                }
            };
            let candidate_path = root_path.join(candidate);
            if candidate_path.is_file() {
                resolved.push(candidate_path);
            }
        }
        resolved
    }
}

/// Resolves a source reference to concrete on-disk target paths, using the
/// annotating file's directory first and then the resolver's configured roots.
pub(super) fn resolve_source_ref_paths(
    source_path: &Path,
    source_ref: &SourceRef,
    resolver: &NativeSourceResolver,
) -> Vec<PathBuf> {
    let candidate = match &source_ref.kind {
        SourceRefKind::Literal(candidate) | SourceRefKind::Directive(candidate) => {
            candidate.as_str()
        }
        SourceRefKind::DirectiveDevNull
        | SourceRefKind::Dynamic
        | SourceRefKind::SingleVariableStaticTail { .. } => return Vec::new(),
    };

    let candidate_path = PathBuf::from(candidate);
    if candidate_path.is_absolute() {
        return candidate_path
            .is_file()
            .then_some(candidate_path)
            .into_iter()
            .collect();
    }

    let mut resolved = Vec::new();
    if let Some(base_dir) = source_path.parent() {
        let direct = base_dir.join(&candidate_path);
        if direct.is_file() {
            resolved.push(direct);
        }
    }
    resolved.extend(resolver.resolve_candidate_paths(source_path, candidate));
    resolved.sort();
    resolved.dedup();
    resolved
}
