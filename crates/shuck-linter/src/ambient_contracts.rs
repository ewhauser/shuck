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
            merge_contract(&mut merged, (provider.build)());
        }
    }

    matched.then_some(merged)
}

fn providers() -> &'static [AmbientContractProvider] {
    &[
        AmbientContractProvider {
            matches: matches_void_packages_build_style_contract,
            build: build_void_packages_build_style_contract,
        },
        AmbientContractProvider {
            matches: matches_void_packages_pre_pkg_hook_contract,
            build: build_void_packages_pre_pkg_hook_contract,
        },
        AmbientContractProvider {
            matches: matches_void_packages_xbps_src_framework_contract,
            build: build_void_packages_xbps_src_framework_contract,
        },
        AmbientContractProvider {
            matches: matches_void_packages_pycompile_trigger_contract,
            build: build_void_packages_pycompile_trigger_contract,
        },
    ]
}

fn merge_contract(merged: &mut FileContract, contract: FileContract) {
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

fn matches_void_packages_build_style_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    (path_matches_any(
        &lower,
        &[
            "void-packages/common/build-style/",
            "void-packages/common/environment/build-style/",
            "void-packages__common__build-style__",
            "void-packages__common__environment__build-style__",
        ],
    )) && has_probable_function_definition(source)
        && source_mentions_any(source, &["wrksrc", "XBPS_SRCPKGDIR"])
}

fn matches_void_packages_pre_pkg_hook_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    path_matches_any(
        &lower,
        &[
            "void-packages/common/hooks/pre-pkg/",
            "void-packages__common__hooks__pre-pkg__",
        ],
    ) && (lower.contains("/99-pkglint") || lower.contains("__99-pkglint"))
        && lower.ends_with(".sh")
        && has_named_function_definition(source, "hook")
        && source.contains("PKGDESTDIR")
}

fn matches_void_packages_xbps_src_framework_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    let libexec_path = path_matches_any(
        &lower,
        &[
            "void-packages/common/xbps-src/libexec/",
            "void-packages__common__xbps-src__libexec__",
        ],
    );
    let shutils_path = path_matches_any(
        &lower,
        &[
            "void-packages/common/xbps-src/shutils/",
            "void-packages__common__xbps-src__shutils__",
        ],
    );
    (libexec_path || shutils_path)
        && lower.ends_with(".sh")
        && xbps_src_framework_has_shell_shape(source, libexec_path)
        && source.matches("XBPS_").count() >= 3
}

fn matches_void_packages_pycompile_trigger_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    (lower.ends_with("/void-packages/srcpkgs/xbps-triggers/files/pycompile")
        || lower.ends_with("__void-packages__srcpkgs__xbps-triggers__files__pycompile"))
        && (source.contains("ACTION=\"$1\"")
            || source.contains("TARGET=\"$2\"")
            || source.contains("case \"$ACTION\""))
}

fn build_void_packages_build_style_contract() -> FileContract {
    variable_contract(&[
        "build_style",
        "distfiles",
        "metapackage",
        "pkgname",
        "pkgver",
        "pycompile_version",
        "XBPS_SRCPKGDIR",
        "XBPS_TARGET_WORDSIZE",
        "configure_args",
        "makejobs",
        "cross_binutils_configure_args",
        "cross_gcc_bootstrap_configure_args",
        "cross_gcc_configure_args",
        "cross_glibc_configure_args",
        "cross_musl_configure_args",
        "wrksrc",
    ])
}

fn build_void_packages_pre_pkg_hook_contract() -> FileContract {
    variable_contract(&[
        "PKGDESTDIR",
        "pkgname",
        "pkgver",
        "metapackage",
        "conf_files",
        "provides",
        "XBPS_COMMONDIR",
        "XBPS_STATEDIR",
        "XBPS_TARGET_MACHINE",
        "XBPS_QUERY_XCMD",
        "XBPS_UHELPER_CMD",
    ])
}

fn build_void_packages_xbps_src_framework_contract() -> FileContract {
    variable_contract(&[
        "XBPS_COMMONDIR",
        "XBPS_SRCPKGDIR",
        "XBPS_BUILDSTYLEDIR",
        "XBPS_LIBEXECDIR",
        "XBPS_STATEDIR",
        "XBPS_MACHINE",
        "XBPS_TARGET",
        "XBPS_TARGET_MACHINE",
        "XBPS_TARGET_PKG",
        "XBPS_CROSS_BUILD",
        "pkgname",
        "pkgver",
        "build_style",
        "sourcepkg",
        "subpackages",
        "NOCOLORS",
        "XBPS_CFLAGS",
        "XBPS_CPPFLAGS",
        "XBPS_CXXFLAGS",
        "XBPS_FFLAGS",
        "XBPS_LDFLAGS",
    ])
}

fn build_void_packages_pycompile_trigger_contract() -> FileContract {
    variable_contract(&["pycompile_dirs", "pycompile_module", "pycompile_version"])
}

fn variable_contract(names: &[&str]) -> FileContract {
    let mut contract = FileContract::default();
    for name in names {
        contract.add_provided_binding(ProvidedBinding::new(
            Name::from(*name),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
    contract
}

fn lower_path(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

fn path_matches_any(lower_path: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| lower_path.contains(pattern))
}

fn has_probable_function_definition(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(probable_function_definition)
}

fn has_named_function_definition(source: &str, name: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(|trimmed| named_function_definition(trimmed, name))
}

fn xbps_src_framework_has_shell_shape(source: &str, libexec_path: bool) -> bool {
    has_probable_function_definition(source)
        || (libexec_path
            && source.contains("readonly XBPS_TARGET")
            && source.contains("setup_pkg \"$PKGNAME\""))
}

fn probable_function_definition(trimmed: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        return rest.contains('{');
    }

    trimmed.contains("() {") || trimmed.contains("(){")
}

fn named_function_definition(trimmed: &str, name: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        let rest = rest.trim_start();
        return rest.starts_with(name) && rest.contains('{');
    }

    trimmed.starts_with(&format!("{name}()"))
        || trimmed.starts_with(&format!("{name} ()"))
        || trimmed.contains(&format!("{name}() {{"))
        || trimmed.contains(&format!("{name}(){{"))
}

fn source_mentions_any(source: &str, names: &[&str]) -> bool {
    names.iter().any(|name| source_mentions_name(source, name))
}

fn source_mentions_name(source: &str, name: &str) -> bool {
    source.contains(&format!("${name}"))
        || source.contains(&format!("${{{name}}}"))
        || source.contains(&format!("${{{name}:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileContextTag, classify_file_context};

    fn contract_for(path: &Path, source: &str) -> Option<FileContract> {
        let context = classify_file_context(source, Some(path), ShellDialect::Sh);
        file_entry_contract(source, Some(path), ShellDialect::Sh, &context)
    }

    fn has_binding(contract: &FileContract, name: &str) -> bool {
        contract
            .provided_bindings
            .iter()
            .any(|binding| binding.name == name)
    }

    #[test]
    fn void_packages_build_style_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/build-style/void-cross.sh");
        let source = "\
helper() { cd \"${wrksrc}\"; }
printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$XBPS_SRCPKGDIR\" \"$configure_args\"
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "pkgname"));
        assert!(has_binding(&contract, "pkgver"));
        assert!(has_binding(&contract, "wrksrc"));
        assert!(has_binding(&contract, "XBPS_SRCPKGDIR"));
        assert!(has_binding(&contract, "configure_args"));
    }

    #[test]
    fn void_packages_pre_pkg_hook_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/hooks/pre-pkg/99-pkglint.sh");
        let source = "\
hook() { printf '%s\\n' \"$PKGDESTDIR\"; }
printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$XBPS_COMMONDIR\"
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "PKGDESTDIR"));
        assert!(has_binding(&contract, "pkgname"));
        assert!(has_binding(&contract, "pkgver"));
        assert!(has_binding(&contract, "XBPS_COMMONDIR"));
    }

    #[test]
    fn void_packages_xbps_src_framework_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/xbps-src/shutils/common.sh");
        let source = "\
helper() { printf '%s\\n' \"$XBPS_COMMONDIR\"; }
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$XBPS_STATEDIR\" \"$pkgname\" \"$build_style\"
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "XBPS_COMMONDIR"));
        assert!(has_binding(&contract, "XBPS_SRCPKGDIR"));
        assert!(has_binding(&contract, "XBPS_STATEDIR"));
        assert!(has_binding(&contract, "build_style"));
    }

    #[test]
    fn void_packages_xbps_src_libexec_drivers_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/xbps-src/libexec/build.sh");
        let source = "\
readonly XBPS_TARGET=\"$1\"
setup_pkg \"$PKGNAME\"
for subpkg in ${subpackages} ${sourcepkg}; do
  printf '%s\\n' \"$XBPS_LIBEXECDIR\" \"$XBPS_CROSS_BUILD\"
done
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "sourcepkg"));
        assert!(has_binding(&contract, "subpackages"));
        assert!(has_binding(&contract, "XBPS_LIBEXECDIR"));
        assert!(has_binding(&contract, "XBPS_CROSS_BUILD"));
    }

    #[test]
    fn void_packages_pycompile_trigger_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/srcpkgs/xbps-triggers/files/pycompile");
        let source = "\
ACTION=\"$1\"
TARGET=\"$2\"
case \"$ACTION\" in
run) printf '%s\\n' \"$pycompile_dirs\" \"$pycompile_module\" \"$pycompile_version\" ;;
esac
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "pycompile_dirs"));
        assert!(has_binding(&contract, "pycompile_module"));
        assert!(has_binding(&contract, "pycompile_version"));
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

    #[test]
    fn void_packages_paths_without_required_source_anchors_do_not_inject_contracts() {
        let xbps_src_path = Path::new("/tmp/void-packages/common/xbps-src/shutils/common.sh");
        let xbps_src_source = "printf '%s\\n' \"$XBPS_COMMONDIR\"\n";
        assert!(contract_for(xbps_src_path, xbps_src_source).is_none());

        let pycompile_path = Path::new("/tmp/void-packages/srcpkgs/xbps-triggers/files/pycompile");
        let pycompile_source = "printf '%s\\n' \"$pycompile_version\"\n";
        assert!(contract_for(pycompile_path, pycompile_source).is_none());
    }

    #[test]
    fn flattened_large_corpus_void_packages_paths_also_get_contracts() {
        let build_style_path =
            Path::new("/tmp/scripts/void-linux__void-packages__common__build-style__void-cross.sh");
        let build_style_source = "\
helper() { :; }
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$configure_args\" \"$wrksrc\"
";
        let build_style_contract = contract_for(build_style_path, build_style_source).unwrap();
        assert!(has_binding(&build_style_contract, "wrksrc"));
        assert!(has_binding(&build_style_contract, "configure_args"));

        let pycompile_path = Path::new(
            "/tmp/scripts/void-linux__void-packages__srcpkgs__xbps-triggers__files__pycompile",
        );
        let pycompile_source = "\
ACTION=\"$1\"
case \"$ACTION\" in
run) printf '%s\\n' \"$pycompile_version\" ;;
esac
";
        let pycompile_contract = contract_for(pycompile_path, pycompile_source).unwrap();
        assert!(has_binding(&pycompile_contract, "pycompile_version"));
    }
}
