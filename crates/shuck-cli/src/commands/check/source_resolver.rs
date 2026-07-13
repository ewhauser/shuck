//! Native resolver for `# shuck: source=` directive target paths.
//!
//! The source closure already resolves a directive path relative to the
//! annotating file's own directory. This resolver adds the configured
//! `[lint] source-paths` roots as additional search directories, mirroring the
//! ShellCheck-compat `--source-path` behavior, and exposes a helper for
//! resolving a `SourceRef` to concrete on-disk targets (used to decide which
//! `lint=true` targets to lint).

use std::path::{Path, PathBuf};

use shuck_semantic::{SourcePathResolver, SourceRef};

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
        // Only the configured roots here; the closure adds the base-directory
        // candidate itself.
        shuck_semantic::resolve_candidate_against_roots(
            source_path,
            candidate,
            &self.source_paths,
            &self.cwd,
        )
    }
}

/// Resolves a source reference to concrete on-disk target paths, using the
/// annotating file's directory first and then the resolver's configured roots.
pub(super) fn resolve_source_ref_paths(
    source_path: &Path,
    source_ref: &SourceRef,
    resolver: &NativeSourceResolver,
) -> Vec<PathBuf> {
    shuck_semantic::resolve_source_ref_targets(
        source_path,
        source_ref,
        &resolver.source_paths,
        &resolver.cwd,
    )
}
