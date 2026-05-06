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

use shuck_ast::{Name, NormalizedCommand};
use shuck_semantic::{FileContract, FileEntryContractCollector};

use crate::ShellDialect;

mod path;
mod source_scan;
mod sourced_runtime;
#[cfg(test)]
mod tests;
mod zsh_caller_arrays;
mod zsh_config;
mod zsh_module_metadata;
mod zsh_paths;
mod zsh_runtime;

struct AmbientContractProvider {
    matches: fn(&AmbientContractCollector<'_>, &Path, ShellDialect) -> bool,
    build: fn(&AmbientContractCollector<'_>, &Path, ShellDialect) -> FileContract,
}

pub(crate) struct AmbientContractCollector<'a> {
    source: &'a str,
    path: Option<&'a Path>,
    shell: ShellDialect,
    completion_initializer_invoked: bool,
    caller_scoped_array_length_names: BTreeSet<Name>,
}

impl<'a> AmbientContractCollector<'a> {
    pub(crate) fn new(source: &'a str, path: Option<&'a Path>, shell: ShellDialect) -> Self {
        let mut caller_scoped_array_length_names = BTreeSet::new();
        if shell == ShellDialect::Zsh {
            zsh_caller_arrays::collect_caller_scoped_array_length_names_from_source(
                source,
                &mut caller_scoped_array_length_names,
            );
        }

        Self {
            source,
            path,
            shell,
            completion_initializer_invoked: false,
            caller_scoped_array_length_names,
        }
    }

    fn file_entry_contract(&self) -> Option<FileContract> {
        let path = self.path?;
        let mut merged = FileContract::default();
        let mut matched = false;

        for provider in providers() {
            if (provider.matches)(self, path, self.shell) {
                matched = true;
                merge_contract(&mut merged, (provider.build)(self, path, self.shell));
            }
        }

        matched.then_some(merged)
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

fn providers() -> &'static [AmbientContractProvider] {
    &[
        AmbientContractProvider {
            matches: sourced_runtime::matches_sourced_runtime_contract,
            build: sourced_runtime::build_sourced_runtime_contract,
        },
        AmbientContractProvider {
            matches: zsh_runtime::matches_zsh_ambient_runtime_contract,
            build: zsh_runtime::build_zsh_ambient_runtime_contract,
        },
        AmbientContractProvider {
            matches: zsh_config::matches_zsh_config_contract,
            build: zsh_config::build_zsh_config_contract,
        },
        AmbientContractProvider {
            matches: zsh_module_metadata::matches_zsh_module_metadata_contract,
            build: zsh_module_metadata::build_zsh_module_metadata_contract,
        },
        AmbientContractProvider {
            matches: zsh_caller_arrays::matches_zsh_caller_scoped_array_contract,
            build: zsh_caller_arrays::build_zsh_caller_scoped_array_contract,
        },
    ]
}

fn merge_contract(merged: &mut FileContract, contract: FileContract) {
    merged.externally_consumed_bindings |= contract.externally_consumed_bindings;
    for name in contract.required_reads {
        merged.add_required_read(name);
    }
    for name in contract.externally_consumed_binding_names {
        merged.add_externally_consumed_binding_name(name);
    }
    for binding in contract.provided_bindings {
        merged.add_provided_binding(binding);
    }
    for function in contract.provided_functions {
        merged.add_provided_function(function);
    }
    for prefix in contract.externally_consumed_binding_prefixes {
        merged.add_externally_consumed_binding_prefix(prefix);
    }
}
