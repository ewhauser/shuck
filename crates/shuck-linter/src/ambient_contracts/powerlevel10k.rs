use shuck_ast::Name;
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use super::AmbientContractCollector;
use super::path::path_matches_any;
use crate::ShellDialect;

const POWERLEVEL10K_BOOTSTRAP_GROUP_A: &[&str] =
    &["__p9k_sourced", "__p9k_root_dir", "__p9k_intro"];
const POWERLEVEL10K_BOOTSTRAP_GROUP_B: &[&str] = &["__p9k_root_dir", "__p9k_intro"];

pub(super) fn matches_powerlevel10k_bootstrap_contract(
    collector: &AmbientContractCollector<'_>,
    shell: ShellDialect,
) -> bool {
    matches!(shell, ShellDialect::Zsh | ShellDialect::Unknown)
        && bootstrap_bindings_for_path(collector.path_signals().lower_path())
            .into_iter()
            .flatten()
            .any(|name| collector.source_signals().mentions_name(name))
}

pub(super) fn apply_powerlevel10k_bootstrap_contract(
    contract: &mut FileContract,
    collector: &AmbientContractCollector<'_>,
) {
    let Some(bindings) = bootstrap_bindings_for_path(collector.path_signals().lower_path()) else {
        return;
    };

    for name in bindings {
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
            Name::from(*name),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
}

pub(super) fn matches_powerlevel10k_gitstatus_contract(
    collector: &AmbientContractCollector<'_>,
    _shell: ShellDialect,
) -> bool {
    let path = collector.path_signals().lower_path();
    let source = collector.source_signals();
    gitstatus_contract_path_shape(path)
        && (source.assigns_name_with_prefix("VCS_STATUS_")
            || (gitstatus_zsh_path_shape(path) && source.mentions_name("__p9k_intro_base")))
}

pub(super) fn apply_powerlevel10k_gitstatus_contract(
    contract: &mut FileContract,
    collector: &AmbientContractCollector<'_>,
) {
    let path = collector.path_signals().lower_path();
    if !gitstatus_contract_path_shape(path) {
        return;
    }

    if collector
        .source_signals()
        .assigns_name_with_prefix("VCS_STATUS_")
    {
        contract.add_externally_consumed_binding_prefix(Name::from("VCS_STATUS_"));
    }

    if gitstatus_zsh_path_shape(path)
        && collector.source_signals().mentions_name("__p9k_intro_base")
    {
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
            Name::from("__p9k_intro_base"),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
}

fn bootstrap_bindings_for_path(lower_path: &str) -> Option<&'static [&'static str]> {
    if lower_path.contains("/powerlevel10k/internal/p10k.zsh") {
        return Some(POWERLEVEL10K_BOOTSTRAP_GROUP_A);
    }

    if path_matches_any(
        lower_path,
        &[
            "/powerlevel10k/internal/configure.zsh",
            "/powerlevel10k/internal/icons.zsh",
            "/powerlevel10k/internal/parser.zsh",
            "/powerlevel10k/internal/worker.zsh",
            "/powerlevel10k/internal/wizard.zsh",
        ],
    ) {
        return Some(POWERLEVEL10K_BOOTSTRAP_GROUP_B);
    }

    None
}

fn gitstatus_contract_path_shape(lower_path: &str) -> bool {
    path_matches_any(
        lower_path,
        &[
            "/powerlevel10k/gitstatus/gitstatus.plugin.sh",
            "/powerlevel10k/gitstatus/gitstatus.plugin.zsh",
        ],
    )
}

fn gitstatus_zsh_path_shape(lower_path: &str) -> bool {
    lower_path.contains("/powerlevel10k/gitstatus/gitstatus.plugin.zsh")
}
