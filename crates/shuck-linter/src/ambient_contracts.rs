use std::path::Path;

use shuck_ast::Name;
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use crate::{FileContext, ShellDialect};

struct AmbientContractProvider {
    matches: fn(source: &str, path: &Path, shell: ShellDialect, file_context: &FileContext) -> bool,
    build: fn() -> FileContract,
}

pub(crate) fn file_entry_contract(
    source: &str,
    path: Option<&Path>,
    shell: ShellDialect,
    file_context: &FileContext,
) -> Option<FileContract> {
    let path = path?;
    let mut merged = FileContract::default();
    let mut matched = false;

    for provider in providers() {
        if (provider.matches)(source, path, shell, file_context) {
            matched = true;
            let contract = (provider.build)();
            for name in contract.required_reads {
                merged.add_required_read(name);
            }
            for binding in contract.provided_bindings {
                merged.add_provided_binding(binding);
            }
            for function in contract.provided_functions {
                merged.add_provided_function(function);
            }
        }
    }

    matched.then_some(merged)
}

fn providers() -> &'static [AmbientContractProvider] {
    &[AmbientContractProvider {
        matches: matches_void_packages_common_contract,
        build: build_void_packages_common_contract,
    }]
}

fn matches_void_packages_common_contract(
    _source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    lower.contains("void-packages/common/")
        && (lower.contains("/build-style/")
            || lower.contains("/hooks/pre-pkg/")
            || lower.ends_with("/pycompile.sh")
            || lower.ends_with("/pycompile")
            || lower.ends_with("/99-pkglint.sh")
            || lower.ends_with("/void-cross.sh"))
}

fn build_void_packages_common_contract() -> FileContract {
    let mut contract = FileContract::default();
    for name in [
        "build_style",
        "distfiles",
        "metapackage",
        "pkgname",
        "pkgver",
        "pycompile_version",
        "wrksrc",
    ] {
        contract.add_provided_binding(ProvidedBinding::new(
            Name::from(name),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
    contract
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileContextTag, classify_file_context};

    #[test]
    fn void_packages_common_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/build-style/void-cross.sh");
        let source = "printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$wrksrc\"\n";
        let context = classify_file_context(source, Some(path), ShellDialect::Sh);

        let contract = file_entry_contract(source, Some(path), ShellDialect::Sh, &context).unwrap();

        assert!(
            contract
                .provided_bindings
                .iter()
                .any(|binding| binding.name == "pkgname")
        );
        assert!(
            contract
                .provided_bindings
                .iter()
                .any(|binding| binding.name == "pkgver")
        );
        assert!(
            contract
                .provided_bindings
                .iter()
                .any(|binding| binding.name == "wrksrc")
        );
    }

    #[test]
    fn broad_project_closure_tags_alone_do_not_inject_contracts() {
        let path = Path::new("/tmp/project/scripts/helper.sh");
        let source = "\
# shellcheck source=helper-lib.sh
. ./helper-lib.sh
printf '%s\\n' \"$pkgname\"
";
        let context = classify_file_context(source, Some(path), ShellDialect::Sh);
        assert!(context.has_tag(FileContextTag::ProjectClosure));

        assert!(file_entry_contract(source, Some(path), ShellDialect::Sh, &context).is_none());
    }
}
