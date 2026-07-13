//! Shared on-disk resolution for `source` references and source hints.
//!
//! Turning a `source`/`.` operand (a literal path, or a `# shuck: source=` /
//! `# shellcheck source=` directive path) into concrete files
//! is needed both by `shuck check` (to decide which `lint=true` targets to
//! lint) and by the language server (to build the cross-file call index). This
//! module is the one implementation both use, so their resolution agrees.

use std::path::{Path, PathBuf};

use crate::{SourceRef, SourceRefKind};

/// Resolves the on-disk target of a single source reference.
///
/// Only *determinable* references resolve: a literal path or a directive path
/// (`# shuck: source=` / `# shellcheck source=`). `/dev/null`
/// directives and unresolvable dynamic paths contribute nothing. The operand
/// names one intended file, so resolution is first-match-wins in precedence
/// order: the annotating file's own directory, then each of `roots` in
/// configured order (relative roots joined onto `root_base`; the token
/// `SCRIPTDIR` maps to the annotating file's directory).
pub fn resolve_source_ref_targets(
    source_path: &Path,
    source_ref: &SourceRef,
    roots: &[String],
    root_base: &Path,
) -> Option<PathBuf> {
    let candidate = match &source_ref.kind {
        SourceRefKind::Literal(candidate) | SourceRefKind::Directive(candidate) => {
            candidate.as_str()
        }
        SourceRefKind::DirectiveDevNull
        | SourceRefKind::Dynamic
        | SourceRefKind::SingleVariableStaticTail { .. } => return None,
    };
    resolve_candidate_targets(source_path, candidate, roots, root_base)
}

/// Resolves a raw candidate path to the first existing on-disk file in
/// precedence order (see [`resolve_source_ref_targets`]). A nearer match
/// always shadows configured-root matches, so a target that exists both next
/// to the annotating file and under a configured root resolves to the local
/// one.
pub fn resolve_candidate_targets(
    source_path: &Path,
    candidate: &str,
    roots: &[String],
    root_base: &Path,
) -> Option<PathBuf> {
    let candidate_path = PathBuf::from(candidate);
    if candidate_path.is_absolute() {
        return candidate_path.is_file().then_some(candidate_path);
    }

    if let Some(base_dir) = source_path.parent() {
        let direct = base_dir.join(&candidate_path);
        if direct.is_file() {
            return Some(direct);
        }
    }
    resolve_candidate_against_roots(source_path, candidate, roots, root_base)
        .into_iter()
        .next()
}

/// Resolves `candidate` against the configured roots only — no base-directory
/// candidate and no absolute short-circuit. Matches are returned in root
/// order, so the first entry is the highest-precedence match. This is the
/// root-search rule shared by [`resolve_candidate_targets`] and
/// closure-facing `SourcePathResolver` implementations (which supply the
/// base-directory candidate themselves).
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn candidate_next_to_annotating_file_shadows_root_matches() {
        let tempdir = tempfile::tempdir().unwrap();
        let base = tempdir.path();
        fs::create_dir_all(base.join("scripts")).unwrap();
        fs::create_dir_all(base.join("lib")).unwrap();
        fs::write(base.join("scripts/util.sh"), "").unwrap();
        fs::write(base.join("lib/util.sh"), "").unwrap();

        let resolved = resolve_candidate_targets(
            &base.join("scripts/main.sh"),
            "util.sh",
            &["lib".to_owned()],
            base,
        );
        assert_eq!(
            resolved,
            Some(base.join("scripts/util.sh")),
            "the annotating file's own directory wins over configured roots"
        );
    }

    #[test]
    fn earlier_configured_root_shadows_later_ones() {
        let tempdir = tempfile::tempdir().unwrap();
        let base = tempdir.path();
        fs::create_dir_all(base.join("first")).unwrap();
        fs::create_dir_all(base.join("second")).unwrap();
        fs::write(base.join("first/util.sh"), "").unwrap();
        fs::write(base.join("second/util.sh"), "").unwrap();

        let resolved = resolve_candidate_targets(
            &base.join("scripts/main.sh"),
            "util.sh",
            &["first".to_owned(), "second".to_owned()],
            base,
        );
        assert_eq!(
            resolved,
            Some(base.join("first/util.sh")),
            "roots are searched in configured order, first match wins"
        );
    }

    #[test]
    fn unresolvable_candidate_yields_none() {
        let tempdir = tempfile::tempdir().unwrap();
        let base = tempdir.path();
        let resolved = resolve_candidate_targets(
            &base.join("scripts/main.sh"),
            "missing.sh",
            &["lib".to_owned()],
            base,
        );
        assert_eq!(resolved, None);
    }
}
