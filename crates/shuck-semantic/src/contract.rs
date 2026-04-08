use rustc_hash::FxHashMap;
use shuck_ast::Name;
use std::path::Path;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProvidedBinding {
    pub name: Name,
    pub kind: ProvidedBindingKind,
    pub certainty: ContractCertainty,
}

impl ProvidedBinding {
    pub fn new(name: Name, kind: ProvidedBindingKind, certainty: ContractCertainty) -> Self {
        Self {
            name,
            kind,
            certainty,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileContract {
    pub required_reads: Vec<Name>,
    pub provided_bindings: Vec<ProvidedBinding>,
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
                merged = true;
                break;
            }
        }

        if !merged {
            self.provided_bindings.push(binding);
        }
    }

    pub(crate) fn merge_candidate_contracts(contracts: &[Self]) -> Self {
        let mut merged = Self::default();
        let total = contracts.len();
        let mut provided_counts: FxHashMap<(Name, ProvidedBindingKind), (usize, bool)> =
            FxHashMap::default();

        for contract in contracts {
            for name in &contract.required_reads {
                merged.add_required_read(name.clone());
            }
            for binding in &contract.provided_bindings {
                let entry = provided_counts
                    .entry((binding.name.clone(), binding.kind))
                    .or_insert((0, true));
                entry.0 += 1;
                entry.1 &= binding.certainty == ContractCertainty::Definite;
            }
        }

        for ((name, kind), (present_count, all_definite)) in provided_counts {
            let certainty = if present_count == total && all_definite {
                ContractCertainty::Definite
            } else {
                ContractCertainty::Possible
            };
            merged.add_provided_binding(ProvidedBinding::new(name, kind, certainty));
        }

        merged
    }
}

#[derive(Clone, Default)]
pub struct SemanticBuildOptions<'a> {
    pub source_path: Option<&'a Path>,
    pub source_path_resolver: Option<&'a (dyn SourcePathResolver + Send + Sync)>,
    pub file_entry_contract: Option<FileContract>,
}
