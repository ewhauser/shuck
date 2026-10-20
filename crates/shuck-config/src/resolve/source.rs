//! Native resolver for `# shuck: source=` directive target paths.
//!
//! The source closure already resolves a directive path relative to the
//! annotating file's own directory. This resolver adds the configured
//! `[lint] source-paths` roots as additional search directories, mirroring the
//! ShellCheck-compat `--source-path` behavior, and exposes a helper for
//! resolving a `SourceRef` to concrete on-disk targets (used by `shuck check`
//! to decide which `lint=true` targets to lint). The CLI and the language
//! server share it so a directive resolves the same way in both.

use std::path::{Path, PathBuf};

use shuck_semantic::{SourcePathResolver, SourceRef};

/// Resolves relative source-hint paths against configured roots.
#[derive(Debug, Clone)]
pub struct NativeSourceResolver {
    cwd: PathBuf,
    source_paths: Vec<String>,
}

impl NativeSourceResolver {
    /// Creates a resolver whose relative `source_paths` resolve against `cwd`,
    /// normally the project root that holds the config file.
    pub fn new(cwd: PathBuf, source_paths: Vec<String>) -> Self {
        Self { cwd, source_paths }
    }

    /// Whether any extra roots are configured. When empty, the closure's own
    /// base-directory resolution is sufficient and this resolver adds nothing.
    pub fn has_roots(&self) -> bool {
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

    fn source_root_base(&self) -> Option<PathBuf> {
        Some(self.cwd.clone())
    }
}

/// Resolves a source reference to its on-disk target, using the annotating
/// file's directory first and then the resolver's configured roots; the
/// nearest match wins.
pub fn resolve_source_ref_paths(
    source_path: &Path,
    source_ref: &SourceRef,
    resolver: &NativeSourceResolver,
) -> Option<PathBuf> {
    shuck_semantic::resolve_source_ref_targets(
        source_path,
        source_ref,
        &resolver.source_paths,
        &resolver.cwd,
    )
}

/// Returns the source reference's possible paths in precedence order,
/// including missing candidates that could shadow the current target later.
pub fn source_ref_candidate_paths(
    source_path: &Path,
    source_ref: &SourceRef,
    resolver: &NativeSourceResolver,
) -> Vec<PathBuf> {
    shuck_semantic::source_ref_candidate_paths(
        source_path,
        source_ref,
        &resolver.source_paths,
        &resolver.cwd,
    )
}
