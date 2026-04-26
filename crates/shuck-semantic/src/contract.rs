use rustc_hash::FxHashMap;
use shuck_ast::{Name, NormalizedCommand};
use shuck_parser::ShellProfile;
use std::path::{Path, PathBuf};

use crate::SourcePathResolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContractCertainty {
    Definite,
    Possible,
}

impl ContractCertainty {
    pub(crate) fn merge_same_site(self, other: Self) -> Self {
        match (self, other) {
            (Self::Definite, Self::Definite) => Self::Definite,
            _ => Self::Possible,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProvidedBindingKind {
    Variable,
    Function,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileEntryBindingInitialization {
    #[default]
    AmbientOnly,
    Initialized,
}

impl FileEntryBindingInitialization {
    fn merge_same_site(self, other: Self) -> Self {
        match (self, other) {
            (Self::Initialized, _) | (_, Self::Initialized) => Self::Initialized,
            (Self::AmbientOnly, Self::AmbientOnly) => Self::AmbientOnly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProvidedBinding {
    pub name: Name,
    pub kind: ProvidedBindingKind,
    pub certainty: ContractCertainty,
    pub file_entry_initialization: FileEntryBindingInitialization,
}

impl ProvidedBinding {
    pub fn new(name: Name, kind: ProvidedBindingKind, certainty: ContractCertainty) -> Self {
        Self {
            name,
            kind,
            certainty,
            file_entry_initialization: FileEntryBindingInitialization::AmbientOnly,
        }
    }

    pub fn new_file_entry_initialized(
        name: Name,
        kind: ProvidedBindingKind,
        certainty: ContractCertainty,
    ) -> Self {
        Self {
            name,
            kind,
            certainty,
            file_entry_initialization: FileEntryBindingInitialization::Initialized,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionContract {
    pub name: Name,
    pub required_reads: Vec<Name>,
    pub provided_bindings: Vec<ProvidedBinding>,
    pub origin_paths: Vec<PathBuf>,
}

impl FunctionContract {
    pub fn new(name: Name) -> Self {
        Self {
            name,
            required_reads: Vec::new(),
            provided_bindings: Vec::new(),
            origin_paths: Vec::new(),
        }
    }

    pub fn add_required_read(&mut self, name: Name) {
        if !self.required_reads.contains(&name) {
            self.required_reads.push(name);
        }
    }

    pub fn add_provided_binding(&mut self, binding: ProvidedBinding) {
        let mut merged = false;
        for existing in &mut self.provided_bindings {
            if existing.name == binding.name && existing.kind == binding.kind {
                existing.certainty = existing.certainty.merge_same_site(binding.certainty);
                existing.file_entry_initialization = existing
                    .file_entry_initialization
                    .merge_same_site(binding.file_entry_initialization);
                merged = true;
                break;
            }
        }

        if !merged {
            self.provided_bindings.push(binding);
        }
    }

    pub fn add_origin_path(&mut self, path: PathBuf) {
        if !self.origin_paths.contains(&path) {
            self.origin_paths.push(path);
        }
    }

    pub(crate) fn merge_candidate_contracts(contracts: &[Self]) -> Option<Self> {
        let first = contracts.first()?;
        let mut merged = Self::new(first.name.clone());
        let total = contracts.len();
        let mut provided_counts: FxHashMap<
            (Name, ProvidedBindingKind),
            (usize, bool, FileEntryBindingInitialization),
        > = FxHashMap::default();

        for contract in contracts {
            for name in &contract.required_reads {
                merged.add_required_read(name.clone());
            }
            for binding in &contract.provided_bindings {
                let entry = provided_counts
                    .entry((binding.name.clone(), binding.kind))
                    .or_insert((0, true, FileEntryBindingInitialization::AmbientOnly));
                entry.0 += 1;
                entry.1 &= binding.certainty == ContractCertainty::Definite;
                entry.2 = entry.2.merge_same_site(binding.file_entry_initialization);
            }
            for path in &contract.origin_paths {
                merged.add_origin_path(path.clone());
            }
        }

        for ((name, kind), (present_count, all_definite, initialization)) in provided_counts {
            let certainty = if present_count == total && all_definite {
                ContractCertainty::Definite
            } else {
                ContractCertainty::Possible
            };
            let mut binding = ProvidedBinding::new(name, kind, certainty);
            binding.file_entry_initialization = initialization;
            merged.add_provided_binding(binding);
        }

        Some(merged)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileContract {
    pub required_reads: Vec<Name>,
    pub provided_bindings: Vec<ProvidedBinding>,
    pub provided_functions: Vec<FunctionContract>,
    pub externally_consumed_bindings: bool,
}

/// Build-time collector for file-entry contracts discovered during semantic traversal.
///
/// Implementors can observe the normalized command stream while the semantic
/// builder is already walking the file, then return a contract that should be
/// applied before final reference resolution, dataflow, and call-graph use.
pub trait FileEntryContractCollector {
    /// Observe one simple command after semantic command normalization.
    fn observe_simple_command(&mut self, command: &NormalizedCommand<'_>);

    /// Return the collected file-entry contract, when this file needs one.
    fn finish(&self) -> Option<FileContract>;
}

impl FileContract {
    pub fn add_required_read(&mut self, name: Name) {
        if !self.required_reads.contains(&name) {
            self.required_reads.push(name);
        }
    }

    pub fn add_provided_binding(&mut self, binding: ProvidedBinding) {
        let mut merged = false;
        for existing in &mut self.provided_bindings {
            if existing.name == binding.name && existing.kind == binding.kind {
                existing.certainty = existing.certainty.merge_same_site(binding.certainty);
                existing.file_entry_initialization = existing
                    .file_entry_initialization
                    .merge_same_site(binding.file_entry_initialization);
                merged = true;
                break;
            }
        }

        if !merged {
            self.provided_bindings.push(binding);
        }
    }

    pub fn add_provided_function(&mut self, function: FunctionContract) {
        let mut merged = false;
        for existing in &mut self.provided_functions {
            if existing.name == function.name {
                for name in &function.required_reads {
                    existing.add_required_read(name.clone());
                }
                for binding in &function.provided_bindings {
                    existing.add_provided_binding(binding.clone());
                }
                for path in &function.origin_paths {
                    existing.add_origin_path(path.clone());
                }
                merged = true;
                break;
            }
        }

        if !merged {
            self.provided_functions.push(function);
        }
    }

    pub(crate) fn merge_candidate_contracts(contracts: &[Self]) -> Self {
        let mut merged = Self::default();
        let total = contracts.len();
        let mut provided_counts: FxHashMap<
            (Name, ProvidedBindingKind),
            (usize, bool, FileEntryBindingInitialization),
        > = FxHashMap::default();
        let mut function_contracts_by_name: FxHashMap<Name, Vec<FunctionContract>> =
            FxHashMap::default();

        for contract in contracts {
            merged.externally_consumed_bindings |= contract.externally_consumed_bindings;
            for name in &contract.required_reads {
                merged.add_required_read(name.clone());
            }
            for binding in &contract.provided_bindings {
                let entry = provided_counts
                    .entry((binding.name.clone(), binding.kind))
                    .or_insert((0, true, FileEntryBindingInitialization::AmbientOnly));
                entry.0 += 1;
                entry.1 &= binding.certainty == ContractCertainty::Definite;
                entry.2 = entry.2.merge_same_site(binding.file_entry_initialization);
            }
            for function in &contract.provided_functions {
                function_contracts_by_name
                    .entry(function.name.clone())
                    .or_default()
                    .push(function.clone());
            }
        }

        for ((name, kind), (present_count, all_definite, initialization)) in provided_counts {
            let certainty = if present_count == total && all_definite {
                ContractCertainty::Definite
            } else {
                ContractCertainty::Possible
            };
            let mut binding = ProvidedBinding::new(name, kind, certainty);
            binding.file_entry_initialization = initialization;
            merged.add_provided_binding(binding);
        }

        for functions in function_contracts_by_name.into_values() {
            if functions.len() != total {
                continue;
            }
            if let Some(function) = FunctionContract::merge_candidate_contracts(&functions) {
                merged.add_provided_function(function);
            }
        }

        merged
    }
}

pub struct SemanticBuildOptions<'a> {
    pub source_path: Option<&'a Path>,
    pub source_path_resolver: Option<&'a (dyn SourcePathResolver + Send + Sync)>,
    pub file_entry_contract: Option<FileContract>,
    pub file_entry_contract_collector: Option<&'a mut dyn FileEntryContractCollector>,
    pub analyzed_paths: Option<&'a rustc_hash::FxHashSet<PathBuf>>,
    pub shell_profile: Option<ShellProfile>,
    pub resolve_source_closure: bool,
}

impl Default for SemanticBuildOptions<'_> {
    fn default() -> Self {
        Self {
            source_path: None,
            source_path_resolver: None,
            file_entry_contract: None,
            file_entry_contract_collector: None,
            analyzed_paths: None,
            shell_profile: None,
            resolve_source_closure: true,
        }
    }
}
