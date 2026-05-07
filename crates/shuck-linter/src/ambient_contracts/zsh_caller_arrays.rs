//! Caller-scoped zsh array length parameters used by sourced helpers.
//!
//! zsh helper functions sometimes test the length of arrays that are supplied by
//! the caller at source time. A typical helper reads only the array length, so the
//! semantic model needs a file-entry binding rather than a local definition:
//!
//! ```zsh
//! safe_rm() {
//!   if [[ ${#dry_run[@]} -gt 0 ]]; then
//!     print -r -- dry
//!   fi
//! }
//! ```
//!
//! The collector gathers these names both from a raw-source fallback and from
//! parsed word parts observed during semantic traversal.

use std::collections::BTreeSet;

use shuck_ast::{
    Name, ParameterExpansionSyntax, Word, WordPart, ZshExpansionOperation, ZshExpansionTarget,
};
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use super::AmbientContractCollector;
use super::source_scan::{code_before_shell_comment, parse_shell_name_at};
use crate::ShellDialect;

pub(super) fn matches_zsh_caller_scoped_array_contract(
    collector: &AmbientContractCollector<'_>,
    shell: ShellDialect,
) -> bool {
    shell == ShellDialect::Zsh
        && collector
            .source_signals()
            .has_probable_function_definition()
        && !collector.caller_scoped_array_length_names.is_empty()
}

pub(super) fn apply_zsh_caller_scoped_array_contract(
    contract: &mut FileContract,
    collector: &AmbientContractCollector<'_>,
) {
    for name in &collector.caller_scoped_array_length_names {
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
            name.clone(),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
}

pub(super) fn collect_caller_scoped_array_length_names(word: &Word, names: &mut BTreeSet<Name>) {
    for part in &word.parts {
        collect_caller_scoped_array_length_names_from_part(&part.kind, names);
    }
}

pub(super) fn collect_caller_scoped_array_length_names_from_source(
    source: &str,
    names: &mut BTreeSet<Name>,
) {
    for line in source.lines() {
        let code = code_before_shell_comment(line);
        collect_caller_scoped_array_length_names_from_code(code, names);
    }
}

fn collect_caller_scoped_array_length_names_from_code(code: &str, names: &mut BTreeSet<Name>) {
    let bytes = code.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = code[cursor..].find("${#") {
        let start = cursor + relative + 3;
        let Some((name, after_name)) = parse_shell_name_at(code, start) else {
            cursor = start;
            continue;
        };
        if bytes.get(after_name) != Some(&b'[')
            || !matches!(bytes.get(after_name + 1), Some(b'@' | b'*'))
            || bytes.get(after_name + 2) != Some(&b']')
            || bytes.get(after_name + 3) != Some(&b'}')
        {
            cursor = after_name;
            continue;
        }
        names.insert(Name::from(name));
        cursor = after_name + 4;
    }
}

fn collect_caller_scoped_array_length_names_from_part(part: &WordPart, names: &mut BTreeSet<Name>) {
    match part {
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                collect_caller_scoped_array_length_names_from_part(&part.kind, names);
            }
        }
        WordPart::ArrayLength(reference) if reference.has_array_selector() => {
            names.insert(reference.name.clone());
        }
        WordPart::Parameter(expansion) => {
            collect_caller_scoped_array_length_names_from_expansion(expansion, names);
        }
        WordPart::ParameterExpansion {
            operand_word_ast, ..
        }
        | WordPart::IndirectExpansion {
            operand_word_ast, ..
        } => {
            if let Some(word) = operand_word_ast {
                collect_caller_scoped_array_length_names(word, names);
            }
        }
        WordPart::Substring {
            offset_word_ast,
            length_word_ast,
            ..
        }
        | WordPart::ArraySlice {
            offset_word_ast,
            length_word_ast,
            ..
        } => {
            collect_caller_scoped_array_length_names(offset_word_ast, names);
            if let Some(word) = length_word_ast {
                collect_caller_scoped_array_length_names(word, names);
            }
        }
        WordPart::ArithmeticExpansion {
            expression_word_ast,
            ..
        } => {
            collect_caller_scoped_array_length_names(expression_word_ast, names);
        }
        _ => {}
    }
}

fn collect_caller_scoped_array_length_names_from_expansion(
    expansion: &shuck_ast::ParameterExpansion,
    names: &mut BTreeSet<Name>,
) {
    match &expansion.syntax {
        ParameterExpansionSyntax::Zsh(syntax) => {
            if syntax.length_prefix.is_some()
                && let ZshExpansionTarget::Reference(reference) = &syntax.target
                && reference.has_array_selector()
            {
                names.insert(reference.name.clone());
            }

            match &syntax.target {
                ZshExpansionTarget::Nested(expansion) => {
                    collect_caller_scoped_array_length_names_from_expansion(expansion, names);
                }
                ZshExpansionTarget::Word(word) => {
                    collect_caller_scoped_array_length_names(word, names);
                }
                ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Empty => {}
            }

            if let Some(operation) = &syntax.operation {
                collect_caller_scoped_array_length_names_from_zsh_operation(operation, names);
            }
            for modifier in &syntax.modifiers {
                if let Some(word) = modifier.argument_word_ast() {
                    collect_caller_scoped_array_length_names(word, names);
                }
            }
        }
        ParameterExpansionSyntax::Bourne(expansion) => {
            if let Some(word) = expansion.operand_word_ast() {
                collect_caller_scoped_array_length_names(word, names);
            }
            if let Some(word) = expansion.offset_word_ast() {
                collect_caller_scoped_array_length_names(word, names);
            }
            if let Some(word) = expansion.length_word_ast() {
                collect_caller_scoped_array_length_names(word, names);
            }
        }
    }
}

fn collect_caller_scoped_array_length_names_from_zsh_operation(
    operation: &ZshExpansionOperation,
    names: &mut BTreeSet<Name>,
) {
    if let Some(word) = operation.operand_word_ast() {
        collect_caller_scoped_array_length_names(word, names);
    }
    if let Some(word) = operation.pattern_word_ast() {
        collect_caller_scoped_array_length_names(word, names);
    }
    if let Some(word) = operation.replacement_word_ast() {
        collect_caller_scoped_array_length_names(word, names);
    }
    if let Some(word) = operation.offset_word_ast() {
        collect_caller_scoped_array_length_names(word, names);
    }
    if let Some(word) = operation.length_word_ast() {
        collect_caller_scoped_array_length_names(word, names);
    }
}
