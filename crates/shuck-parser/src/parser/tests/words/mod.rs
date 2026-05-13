use super::*;

fn word_part_tree_contains_variable(parts: &[WordPartNode], expected: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Variable(name) => name == expected,
        WordPart::DoubleQuoted { parts, .. } => word_part_tree_contains_variable(parts, expected),
        _ => false,
    })
}

fn collect_bourne_parameter_names(parts: &[WordPartNode], names: &mut Vec<String>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_bourne_parameter_names(parts, names),
            WordPart::Parameter(parameter) => {
                let name = match &parameter.syntax {
                    ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                        reference,
                    })
                    | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length {
                        reference,
                    })
                    | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indices {
                        reference,
                    })
                    | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indirect {
                        reference,
                        ..
                    })
                    | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                        reference,
                        ..
                    })
                    | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
                        reference,
                        ..
                    })
                    | ParameterExpansionSyntax::Bourne(
                        BourneParameterExpansion::Transformation { reference, .. },
                    ) => Some(reference.name.to_string()),
                    ParameterExpansionSyntax::Bourne(BourneParameterExpansion::PrefixMatch {
                        prefix,
                        ..
                    }) => Some(prefix.to_string()),
                    ParameterExpansionSyntax::Zsh(_) => None,
                };
                if let Some(name) = name {
                    names.push(name);
                }
            }
            _ => {}
        }
    }
}

fn collect_bourne_parameter_trim_patterns(
    parts: &[WordPartNode],
    source: &str,
    patterns: &mut Vec<String>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_bourne_parameter_trim_patterns(parts, source, patterns);
            }
            WordPart::Parameter(parameter) => {
                if let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                    operator,
                    ..
                }) = &parameter.syntax
                    && let ParameterOp::RemovePrefixShort { pattern }
                    | ParameterOp::RemovePrefixLong { pattern }
                    | ParameterOp::RemoveSuffixShort { pattern }
                    | ParameterOp::RemoveSuffixLong { pattern } = operator.as_ref()
                {
                    patterns.push(pattern.render(source));
                }
            }
            _ => {}
        }
    }
}

fn first_command_substitution_body(parts: &[WordPartNode]) -> Option<&StmtSeq> {
    parts.iter().find_map(|part| match &part.kind {
        WordPart::CommandSubstitution { body, .. } => Some(body),
        WordPart::DoubleQuoted { parts, .. } => first_command_substitution_body(parts),
        _ => None,
    })
}

mod assignments;
mod decode;
mod expansions;
mod integration;
mod patterns;
mod zsh_extensions;
mod zsh_glob;
mod zsh_parameter;
