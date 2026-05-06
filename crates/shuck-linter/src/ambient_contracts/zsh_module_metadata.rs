//! External consumption of zsh module metadata declarations.
//!
//! Some zsh module systems discover modules by reading a small metadata triplet
//! and then invoking the configured entry function. The variables look unused
//! inside the file, but the loader reads them after sourcing:
//!
//! ```zsh
//! module_name="package-manager"
//! module_description="Install package manager"
//! module_main_function="run_package_manager_module"
//!
//! run_package_manager_module() {
//!   :
//! }
//! ```

use shuck_ast::Name;
use shuck_semantic::FileContract;

use super::AmbientContractCollector;
use super::signals::SourceSignals;
use crate::ShellDialect;

pub(super) fn matches_zsh_module_metadata_contract(
    collector: &AmbientContractCollector<'_>,
    shell: ShellDialect,
) -> bool {
    shell == ShellDialect::Zsh && zsh_module_metadata_source_shape(collector.source_signals())
}

pub(super) fn build_zsh_module_metadata_contract(
    _collector: &AmbientContractCollector<'_>,
    _shell: ShellDialect,
) -> FileContract {
    let mut contract = FileContract::default();
    for name in ZSH_MODULE_METADATA_NAMES {
        contract.add_externally_consumed_binding_name(Name::from(*name));
    }
    contract
}

const ZSH_MODULE_METADATA_NAMES: &[&str] =
    &["module_name", "module_description", "module_main_function"];

fn zsh_module_metadata_source_shape(source: &SourceSignals<'_>) -> bool {
    ZSH_MODULE_METADATA_NAMES
        .iter()
        .all(|name| source.assigns_name(name))
        && source
            .static_assignment_value("module_main_function")
            .is_some_and(|function_name| source.defines_function(&function_name))
}
