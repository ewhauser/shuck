use rustc_hash::FxHashMap;
use shuck_ast::{Name, NormalizedCommand};
use shuck_parser::ShellProfile;
use std::path::{Path, PathBuf};

use crate::SourcePathResolver;

/// Confidence attached to a provided binding or function contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContractCertainty {
    /// The contract is guaranteed along the observed path.
    Definite,
    /// The contract may apply, but not on every path.
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

/// Kind of binding described by a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProvidedBindingKind {
    /// A variable binding.
    Variable,
    /// A function binding.
    Function,
}

/// Whether a file-entry provided binding is definitely initialized on entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileEntryBindingInitialization {
    /// The binding is ambiently available but not guaranteed initialized.
    #[default]
    AmbientOnly,
    /// The binding is definitely initialized at file entry.
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

/// One binding provided by an imported file or ambient contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProvidedBinding {
    /// Provided binding name.
    pub name: Name,
    /// Provided binding kind.
    pub kind: ProvidedBindingKind,
    /// Confidence attached to the binding.
    pub certainty: ContractCertainty,
    /// Whether the binding is initialized at file entry.
    pub file_entry_initialization: FileEntryBindingInitialization,
}

impl ProvidedBinding {
    /// Creates a provided binding that is ambiently available but not definitely initialized.
    pub fn new(name: Name, kind: ProvidedBindingKind, certainty: ContractCertainty) -> Self {
        Self {
            name,
            kind,
            certainty,
            file_entry_initialization: FileEntryBindingInitialization::AmbientOnly,
        }
    }

    /// Creates a provided binding that is definitely initialized at file entry.
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

/// Contract for one provided function.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionContract {
    /// Function name.
    pub name: Name,
    /// Names the function is expected to read from its caller environment.
    pub required_reads: Vec<Name>,
    /// Bindings the function may provide to its caller or nested analysis.
    pub provided_bindings: Vec<ProvidedBinding>,
    /// Source files that contributed this function contract.
    pub origin_paths: Vec<PathBuf>,
}

impl FunctionContract {
    /// Creates an empty contract for `name`.
    pub fn new(name: Name) -> Self {
        Self {
            name,
            required_reads: Vec::new(),
            provided_bindings: Vec::new(),
            origin_paths: Vec::new(),
        }
    }

    /// Adds a required read if it has not already been recorded.
    pub fn add_required_read(&mut self, name: Name) {
        if !self.required_reads.contains(&name) {
            self.required_reads.push(name);
        }
    }

    /// Adds or merges a provided binding into this function contract.
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

    /// Records a contributing origin path if it has not already been seen.
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

/// Aggregate contract applied at file entry before final semantic resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileContract {
    /// Names required from the ambient environment before the file runs.
    pub required_reads: Vec<Name>,
    /// Bindings provided by entering the file.
    pub provided_bindings: Vec<ProvidedBinding>,
    /// Functions provided by entering the file.
    pub provided_functions: Vec<FunctionContract>,
    /// Whether the file may consume bindings through external runtime behavior.
    pub externally_consumed_bindings: bool,
    /// Binding name prefixes consumed through external runtime behavior.
    pub externally_consumed_binding_prefixes: Vec<Name>,
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
    /// Adds a required read if it has not already been recorded.
    pub fn add_required_read(&mut self, name: Name) {
        if !self.required_reads.contains(&name) {
            self.required_reads.push(name);
        }
    }

    /// Adds or merges a provided binding into the file contract.
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

    /// Marks a binding name prefix as externally consumed by this file.
    pub fn add_externally_consumed_binding_prefix(&mut self, prefix: Name) {
        if !self
            .externally_consumed_binding_prefixes
            .iter()
            .any(|existing| existing == &prefix)
        {
            self.externally_consumed_binding_prefixes.push(prefix);
        }
    }

    /// Adds or merges a provided function into the file contract.
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
            for prefix in &contract.externally_consumed_binding_prefixes {
                merged.add_externally_consumed_binding_prefix(prefix.clone());
            }
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

/// Options that control how a [`crate::SemanticModel`] is built.
pub struct SemanticBuildOptions<'a> {
    /// Path of the file being analyzed, used for source-closure resolution and diagnostics.
    pub source_path: Option<&'a Path>,
    /// Resolver for mapping source-like paths to candidate tracked files.
    pub source_path_resolver: Option<&'a (dyn SourcePathResolver + Send + Sync)>,
    /// Precomputed file-entry contract to apply before analysis.
    pub file_entry_contract: Option<FileContract>,
    /// Optional observer that can derive a file-entry contract during traversal.
    pub file_entry_contract_collector: Option<&'a mut dyn FileEntryContractCollector>,
    /// Paths already analyzed in the current source-closure walk.
    pub analyzed_paths: Option<&'a rustc_hash::FxHashSet<PathBuf>>,
    /// Explicit shell profile to use instead of inferring one from the source.
    pub shell_profile: Option<ShellProfile>,
    /// Whether sourced-file closure metadata should be resolved and imported.
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
