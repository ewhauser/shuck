//! Ambient contracts describe shell names that exist because a file runs inside
//! a larger shell runtime rather than as a standalone script.
//!
//! The semantic model normally treats a file entry as the start of the world: a
//! read such as `$cur` or an assignment such as `reply=(...)` is judged using the
//! definitions visible in that file and its resolved source closure. Some common
//! shell ecosystems intentionally violate that model. A bash completion helper
//! may call `_init_completion` and then read names initialized by bash-completion:
//!
//! ```sh
//! _example() {
//!   _init_completion || return
//!   printf '%s\n' "$cur" "$cword"
//! }
//! ```
//!
//! A zsh plugin may read special parameters populated by the interactive zsh
//! runtime, or it may assign hook arrays that zsh consumes after the file has
//! been sourced:
//!
//! ```zsh
//! precmd_functions+=(_example_precmd)
//! print -r -- "$history" "$widgets"
//! ```
//!
//! This module recognizes those ecosystem-specific entry conditions and turns
//! them into `FileContract`s. The collector is threaded through semantic build so
//! it can observe normalized simple commands during the existing semantic walk;
//! when semantic build finishes, the collected contract is applied before final
//! reference resolution, dataflow, and call-graph consumers use the model.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use shuck_ast::{Name, NormalizedCommand};
use shuck_parser::ShellProfile;
use shuck_semantic::{FileContract, FileEntryContractCollector, FileEntryContractCollectorFactory};

use crate::ShellDialect;

mod contracts;
mod path;
mod powerlevel10k;
mod signals;
mod source_scan;
mod sourced_runtime;
#[cfg(test)]
mod tests;
mod zsh_caller_arrays;
mod zsh_config;
mod zsh_module_metadata;
mod zsh_paths;
mod zsh_runtime;

pub use contracts::{
    AmbientContractActivation, AmbientContractConfig, AmbientContractEffects, AmbientContractSpec,
    AmbientFunctionContractSpec, EffectiveAmbientContracts, ResolvedAmbientContracts,
    ResolvedAmbientRequestContracts,
};

pub(crate) struct AmbientContractCollector<'a> {
    source: &'a str,
    shell: ShellDialect,
    signals: signals::AmbientSignals<'a>,
    contracts: Arc<ResolvedAmbientContracts>,
    completion_initializer_invoked: bool,
    caller_scoped_array_length_names: BTreeSet<Name>,
}

pub(crate) struct AmbientContractCollectorFactory {
    contracts: Arc<ResolvedAmbientContracts>,
}

impl FileEntryContractCollectorFactory for AmbientContractCollectorFactory {
    fn collector_for_file<'a>(
        &self,
        source: &'a str,
        path: Option<&'a Path>,
        shell_profile: &ShellProfile,
    ) -> Option<Box<dyn FileEntryContractCollector + 'a>> {
        Some(Box::new(AmbientContractCollector::new(
            source,
            path,
            shell_dialect_from_profile(shell_profile),
            Arc::clone(&self.contracts),
        )))
    }
}

impl<'a> AmbientContractCollector<'a> {
    pub(crate) fn new(
        source: &'a str,
        path: Option<&'a Path>,
        shell: ShellDialect,
        contracts: Arc<ResolvedAmbientContracts>,
    ) -> Self {
        let mut caller_scoped_array_length_names = BTreeSet::new();
        if shell == ShellDialect::Zsh {
            zsh_caller_arrays::collect_caller_scoped_array_length_names_from_source(
                source,
                &mut caller_scoped_array_length_names,
            );
        }

        Self {
            source,
            shell,
            signals: signals::AmbientSignals::new(source, path),
            contracts,
            completion_initializer_invoked: false,
            caller_scoped_array_length_names,
        }
    }

    fn file_entry_contract(&self) -> Option<FileContract> {
        self.signals.path()?;
        self.contracts.file_entry_contract(self, self.shell)
    }

    fn source_signals(&self) -> &signals::SourceSignals<'a> {
        self.signals.source()
    }

    fn path_signals(&self) -> &signals::PathSignals {
        self.signals
            .path()
            .expect("ambient contracts are only built for path-backed files")
    }
}

impl AmbientContractCollectorFactory {
    pub(crate) fn new(contracts: Arc<ResolvedAmbientContracts>) -> Self {
        Self { contracts }
    }
}

impl FileEntryContractCollector for AmbientContractCollector<'_> {
    fn observe_simple_command(&mut self, command: &NormalizedCommand<'_>) {
        self.completion_initializer_invoked |=
            sourced_runtime::normalized_command_invokes_completion_initializer(
                command,
                self.source,
            );
        if self.shell == ShellDialect::Zsh {
            for word in command.body_args() {
                zsh_caller_arrays::collect_caller_scoped_array_length_names(
                    word,
                    &mut self.caller_scoped_array_length_names,
                );
            }
        }
    }

    fn finish(&self) -> Option<FileContract> {
        self.file_entry_contract()
    }
}

fn shell_dialect_from_profile(profile: &ShellProfile) -> ShellDialect {
    match profile.dialect {
        shuck_parser::ShellDialect::Zsh => ShellDialect::Zsh,
        shuck_parser::ShellDialect::Mksh => ShellDialect::Mksh,
        shuck_parser::ShellDialect::Posix => ShellDialect::Sh,
        shuck_parser::ShellDialect::Bash => ShellDialect::Bash,
    }
}
