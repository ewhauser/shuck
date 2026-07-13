//! Shared on-disk resolution for `source` references and source hints.
//!
//! Turning a `source`/`.` operand (a literal path, or a `# shuck: source=` /
//! `# shellcheck source=` directive path) into concrete files
//! is needed both by `shuck check` (to decide which `lint=true` targets to
//! lint) and by the language server (to build the cross-file call index). This
//! module is the one implementation both use, so their resolution agrees.

use std::path::{Path, PathBuf};

use crate::{SourceRef, SourceRefKind};

/// Resolves the on-disk targets of a single source reference.
///
/// Only *determinable* references resolve: a literal path or a directive path
/// (`# shuck: source=` / `# shellcheck source=`). `/dev/null`
/// directives and unresolvable dynamic paths contribute nothing. The operand is
/// resolved against the annotating file's own directory first, then each of
/// `roots` (relative roots joined onto `root_base`; the token `SCRIPTDIR` maps
/// to the annotating file's directory).
pub fn resolve_source_ref_targets(
    source_path: &Path,
    source_ref: &SourceRef,
    roots: &[String],
    root_base: &Path,
) -> Vec<PathBuf> {
    let candidate = match &source_ref.kind {
        SourceRefKind::Literal(candidate) | SourceRefKind::Directive(candidate) => {
            candidate.as_str()
        }
        SourceRefKind::DirectiveDevNull
        | SourceRefKind::Dynamic
        | SourceRefKind::SingleVariableStaticTail { .. } => return Vec::new(),
    };
    resolve_candidate_targets(source_path, candidate, roots, root_base)
}

/// Resolves a raw candidate path to existing on-disk files (see
/// [`resolve_source_ref_targets`] for the search order).
pub fn resolve_candidate_targets(
    source_path: &Path,
    candidate: &str,
    roots: &[String],
    root_base: &Path,
) -> Vec<PathBuf> {
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
    resolved.extend(resolve_candidate_against_roots(
        source_path,
        candidate,
        roots,
        root_base,
    ));
    resolved.sort();
    resolved.dedup();
    resolved
}

/// Resolves `candidate` against the configured roots only — no base-directory
/// candidate and no absolute short-circuit. This is the root-search rule
/// shared by [`resolve_candidate_targets`] and closure-facing
/// `SourcePathResolver` implementations (which supply the base-directory
/// candidate themselves).
pub fn resolve_candidate_against_roots(
    source_path: &Path,
    candidate: &str,
    roots: &[String],
    root_base: &Path,
) -> Vec<PathBuf> {
    let candidate_path = Path::new(candidate);
    let mut resolved = Vec::new();
    for root in roots {
        let root_path = if root == "SCRIPTDIR" {
            source_path.parent().unwrap_or(Path::new("")).to_path_buf()
        } else {
            let root_path = PathBuf::from(root);
            if root_path.is_absolute() {
                root_path
            } else {
                root_base.join(root_path)
            }
        };
        let joined = root_path.join(candidate_path);
        if joined.is_file() {
            resolved.push(joined);
        }
    }
    resolved
}
