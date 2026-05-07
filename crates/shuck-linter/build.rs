use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(test))]
use std::env;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RuleMetadata {
    new_code: String,
    shellcheck_code: Option<String>,
    shellcheck_level: Option<String>,
    description: String,
    rationale: String,
    fix_description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellCheckLevel {
    Style,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ContractDocument {
    version: u32,
    contracts: Vec<DeclarativeContract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeContract {
    id: String,
    #[serde(default)]
    groups: Vec<String>,
    label: Option<String>,
    when: DeclarativeContractWhen,
    #[serde(default)]
    files: Vec<String>,
    effects: DeclarativeContractEffects,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeContractWhen {
    #[serde(rename = "type")]
    activation_type: DeclarativeContractActivationType,
    framework: Option<String>,
    plugin: Option<String>,
    theme: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum DeclarativeContractActivationType {
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "zsh_plugin")]
    ZshPlugin,
    #[serde(rename = "zsh_theme")]
    ZshTheme,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeContractEffects {
    reads: Vec<String>,
    consumes: DeclarativeContractConsumes,
    provides: DeclarativeContractProvides,
    functions: Vec<DeclarativeFunctionEffects>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeContractConsumes {
    names: Vec<String>,
    prefixes: Vec<String>,
    all: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeContractProvides {
    variables: Vec<String>,
    functions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeFunctionEffects {
    name: String,
    #[serde(default)]
    reads: Vec<String>,
    #[serde(default)]
    sets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedDeclarativeContract {
    id: String,
    groups: Vec<String>,
    label: Option<String>,
    activation: NormalizedDeclarativeActivation,
    files: Vec<String>,
    effects: NormalizedDeclarativeEffects,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NormalizedDeclarativeActivation {
    Always,
    ZshPlugin { framework: String, plugin: String },
    ZshTheme { framework: String, theme: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedDeclarativeEffects {
    reads: Vec<String>,
    consumes_names: Vec<String>,
    consumes_prefixes: Vec<String>,
    consumes_all: bool,
    provides_variables: Vec<String>,
    provides_functions: Vec<String>,
    functions: Vec<NormalizedDeclarativeFunctionEffects>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedDeclarativeFunctionEffects {
    name: String,
    reads: Vec<String>,
    sets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuleMetadataRow {
    code: String,
    description: String,
    rationale: String,
    fix_description: Option<String>,
    shellcheck_level: Option<ShellCheckLevel>,
}

fn parse_shellcheck_code_value(raw: &str) -> Result<Option<u32>, String> {
    let raw = raw.trim().trim_matches(|ch| matches!(ch, '"' | '\''));
    if raw.is_empty() || raw.eq_ignore_ascii_case("null") || raw == "~" {
        return Ok(None);
    }

    let digits = raw
        .strip_prefix("SC")
        .or_else(|| raw.strip_prefix("sc"))
        .ok_or_else(|| "invalid shellcheck_code prefix".to_owned())?;
    let number = digits
        .parse::<u32>()
        .map_err(|_| "invalid shellcheck_code number".to_owned())?;
    Ok(Some(number))
}

fn parse_shellcheck_level_value(raw: &str) -> Result<Option<ShellCheckLevel>, String> {
    let raw = raw.trim().trim_matches(|ch| matches!(ch, '"' | '\''));
    if raw.is_empty() || raw.eq_ignore_ascii_case("null") || raw == "~" {
        return Ok(None);
    }

    match raw.to_ascii_lowercase().as_str() {
        "style" => Ok(Some(ShellCheckLevel::Style)),
        "info" => Ok(Some(ShellCheckLevel::Info)),
        "warning" => Ok(Some(ShellCheckLevel::Warning)),
        "error" => Ok(Some(ShellCheckLevel::Error)),
        _ => Err("invalid shellcheck_level".to_owned()),
    }
}

fn parse_rule_metadata(
    data: &str,
) -> Result<(RuleMetadata, Option<u32>, Option<ShellCheckLevel>), String> {
    let metadata: RuleMetadata =
        serde_yaml::from_str(data).map_err(|err| format!("invalid rule metadata: {err}"))?;

    let rule_code = metadata.new_code.trim();
    if rule_code.is_empty() {
        return Err("missing new_code".to_owned());
    }

    let shellcheck_code = metadata
        .shellcheck_code
        .as_deref()
        .map(parse_shellcheck_code_value)
        .transpose()?
        .flatten();

    let shellcheck_level = metadata
        .shellcheck_level
        .as_deref()
        .map(parse_shellcheck_level_value)
        .transpose()?
        .flatten();

    if shellcheck_code.is_some() && shellcheck_level.is_none() {
        return Err("shellcheck_level must be set when shellcheck_code is set".to_owned());
    }

    Ok((metadata, shellcheck_code, shellcheck_level))
}

fn parse_declarative_contract_document(data: &str) -> Result<ContractDocument, String> {
    serde_yaml::from_str(data).map_err(|err| format!("invalid declarative contracts: {err}"))
}

fn validate_declarative_contract_document(
    data: &str,
    source_path: &Path,
) -> Result<Vec<NormalizedDeclarativeContract>, String> {
    let document = parse_declarative_contract_document(data)?;
    if document.version != 1 {
        return Err(format!(
            "unsupported declarative contract version {} in {}",
            document.version,
            source_path.display()
        ));
    }

    let mut normalized = Vec::new();
    for contract in document.contracts {
        normalized.push(normalize_declarative_contract(contract, source_path)?);
    }

    Ok(normalized)
}

fn normalize_declarative_contract(
    contract: DeclarativeContract,
    source_path: &Path,
) -> Result<NormalizedDeclarativeContract, String> {
    let id = normalize_selector_token("contract id", &contract.id, source_path)?;
    let groups = normalize_selector_tokens("contract group", &contract.groups, source_path)?;
    if groups.is_empty() {
        return Err(format!(
            "missing groups for declarative contract {id:?} in {}",
            source_path.display()
        ));
    }
    if groups.iter().any(|group| group == &id) {
        return Err(format!(
            "contract group must not repeat id {id:?} in {}",
            source_path.display()
        ));
    }

    let activation = normalize_declarative_activation(&contract.when, source_path)?;
    let files = normalize_globs(&contract.files, source_path)?;
    let effects = normalize_declarative_effects(&contract.effects, source_path)?;
    if effects_is_empty(&effects) {
        return Err(format!(
            "declarative contract {id:?} in {} must define at least one effect",
            source_path.display()
        ));
    }

    Ok(NormalizedDeclarativeContract {
        id,
        groups,
        label: contract.label.map(|label| label.trim().to_owned()),
        activation,
        files,
        effects,
    })
}

fn normalize_declarative_activation(
    when: &DeclarativeContractWhen,
    source_path: &Path,
) -> Result<NormalizedDeclarativeActivation, String> {
    match when.activation_type {
        DeclarativeContractActivationType::Always => {
            if when.framework.is_some() || when.plugin.is_some() || when.theme.is_some() {
                return Err(format!(
                    "`always` activation cannot include framework, plugin, or theme in {}",
                    source_path.display()
                ));
            }
            Ok(NormalizedDeclarativeActivation::Always)
        }
        DeclarativeContractActivationType::ZshPlugin => {
            let framework = normalize_nonempty_text(
                "activation framework",
                when.framework.as_deref(),
                source_path,
            )?;
            let plugin =
                normalize_nonempty_text("activation plugin", when.plugin.as_deref(), source_path)?;
            if when.theme.is_some() {
                return Err(format!(
                    "`zsh_plugin` activation cannot include `theme` in {}",
                    source_path.display()
                ));
            }
            Ok(NormalizedDeclarativeActivation::ZshPlugin { framework, plugin })
        }
        DeclarativeContractActivationType::ZshTheme => {
            let framework = normalize_nonempty_text(
                "activation framework",
                when.framework.as_deref(),
                source_path,
            )?;
            let theme =
                normalize_nonempty_text("activation theme", when.theme.as_deref(), source_path)?;
            if when.plugin.is_some() {
                return Err(format!(
                    "`zsh_theme` activation cannot include `plugin` in {}",
                    source_path.display()
                ));
            }
            Ok(NormalizedDeclarativeActivation::ZshTheme { framework, theme })
        }
    }
}

fn normalize_declarative_effects(
    effects: &DeclarativeContractEffects,
    source_path: &Path,
) -> Result<NormalizedDeclarativeEffects, String> {
    Ok(NormalizedDeclarativeEffects {
        reads: normalize_shell_names("reads", &effects.reads, source_path)?,
        consumes_names: normalize_shell_names(
            "consumes.names",
            &effects.consumes.names,
            source_path,
        )?,
        consumes_prefixes: normalize_shell_prefixes(
            "consumes.prefixes",
            &effects.consumes.prefixes,
            source_path,
        )?,
        consumes_all: effects.consumes.all,
        provides_variables: normalize_shell_names(
            "provides.variables",
            &effects.provides.variables,
            source_path,
        )?,
        provides_functions: normalize_shell_names(
            "provides.functions",
            &effects.provides.functions,
            source_path,
        )?,
        functions: effects
            .functions
            .iter()
            .map(|function| normalize_declarative_function_effects(function, source_path))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn normalize_declarative_function_effects(
    function: &DeclarativeFunctionEffects,
    source_path: &Path,
) -> Result<NormalizedDeclarativeFunctionEffects, String> {
    Ok(NormalizedDeclarativeFunctionEffects {
        name: normalize_shell_name("function name", &function.name, source_path)?,
        reads: normalize_shell_names("functions[].reads", &function.reads, source_path)?,
        sets: normalize_shell_names("functions[].sets", &function.sets, source_path)?,
    })
}

fn normalize_selector_tokens(
    field: &str,
    values: &[String],
    source_path: &Path,
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for value in values {
        normalized.push(normalize_selector_token(field, value, source_path)?);
    }
    Ok(unique_stable(normalized))
}

fn normalize_selector_token(
    field: &str,
    value: &str,
    source_path: &Path,
) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{field} must not be empty in {}",
            source_path.display()
        ));
    }
    if trimmed != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace in {}",
            source_path.display()
        ));
    }
    if value.chars().any(char::is_whitespace) {
        return Err(format!(
            "{field} must not contain whitespace in {}",
            source_path.display()
        ));
    }
    Ok(value.to_owned())
}

fn normalize_nonempty_text(
    field: &str,
    value: Option<&str>,
    source_path: &Path,
) -> Result<String, String> {
    let Some(value) = value else {
        return Err(format!("missing {field} in {}", source_path.display()));
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{field} must not be empty in {}",
            source_path.display()
        ));
    }
    if trimmed != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace in {}",
            source_path.display()
        ));
    }
    Ok(value.to_owned())
}

fn normalize_globs(values: &[String], source_path: &Path) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "file glob must not be empty in {}",
                source_path.display()
            ));
        }
        if trimmed != value {
            return Err(format!(
                "file glob must not have leading or trailing whitespace in {}",
                source_path.display()
            ));
        }
        let matcher = value.strip_prefix('!').unwrap_or(value.as_str());
        globset::Glob::new(matcher).map_err(|err| {
            format!(
                "invalid declarative contract glob {value:?} in {}: {err}",
                source_path.display()
            )
        })?;
        normalized.push(value.to_owned());
    }
    Ok(unique_stable(normalized))
}

fn normalize_shell_names(
    field: &str,
    values: &[String],
    source_path: &Path,
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for value in values {
        normalized.push(normalize_shell_name(field, value, source_path)?);
    }
    Ok(unique_stable(normalized))
}

fn normalize_shell_name(field: &str, value: &str, source_path: &Path) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{field} must not be empty in {}",
            source_path.display()
        ));
    }
    if trimmed != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace in {}",
            source_path.display()
        ));
    }
    if !is_portable_shell_identifier(value) {
        return Err(format!(
            "{field} must use a portable shell identifier in {}: {value:?}",
            source_path.display()
        ));
    }
    Ok(value.to_owned())
}

fn normalize_shell_prefixes(
    field: &str,
    values: &[String],
    source_path: &Path,
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "{field} must not be empty in {}",
                source_path.display()
            ));
        }
        if trimmed != value {
            return Err(format!(
                "{field} must not have leading or trailing whitespace in {}",
                source_path.display()
            ));
        }
        if !is_portable_shell_identifier_prefix(value) {
            return Err(format!(
                "{field} must use a portable shell identifier prefix in {}: {value:?}",
                source_path.display()
            ));
        }
        normalized.push(value.to_owned());
    }
    Ok(unique_stable(normalized))
}

fn is_portable_shell_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_portable_shell_identifier_prefix(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn unique_stable(values: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn effects_is_empty(effects: &NormalizedDeclarativeEffects) -> bool {
    effects.reads.is_empty()
        && effects.consumes_names.is_empty()
        && effects.consumes_prefixes.is_empty()
        && !effects.consumes_all
        && effects.provides_variables.is_empty()
        && effects.provides_functions.is_empty()
        && effects.functions.is_empty()
}

fn collect_yaml_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let metadata = fs::symlink_metadata(dir)
        .map_err(|err| format!("read metadata for {}: {err}", dir.display()))?;
    if !metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    let mut paths = Vec::new();
    collect_yaml_files_recursive(dir, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_yaml_files_recursive(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|err| format!("read {}: {err}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_yaml_files_recursive(&path, paths)?;
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "yaml") {
            paths.push(path);
        }
    }
    Ok(())
}

fn load_rule_metadata(
    docs_dir: &Path,
) -> Result<(Vec<(String, u32)>, Vec<RuleMetadataRow>), String> {
    let mut entries = fs::read_dir(docs_dir)
        .map_err(|err| format!("read {}: {err}", docs_dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect::<Vec<_>>();
    entries.sort();

    let mut mappings = Vec::new();
    let mut metadata_rows = Vec::new();
    for path in entries {
        let data =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let (metadata, shellcheck_code, shellcheck_level) =
            parse_rule_metadata(&data).map_err(|err| format!("{err} in {}", path.display()))?;
        let rule_code = metadata.new_code.trim().to_owned();
        metadata_rows.push(RuleMetadataRow {
            code: rule_code.clone(),
            description: metadata.description,
            rationale: metadata.rationale,
            fix_description: metadata.fix_description,
            shellcheck_level,
        });
        if let Some(shellcheck_code) = shellcheck_code {
            mappings.push((rule_code, shellcheck_code));
        }
    }
    Ok((mappings, metadata_rows))
}

fn load_declarative_contracts(
    contracts_dir: &Path,
) -> Result<Vec<NormalizedDeclarativeContract>, String> {
    let yaml_files = collect_yaml_files(contracts_dir)?;
    let mut contracts = Vec::new();
    let mut seen_ids = HashSet::new();

    for path in yaml_files {
        let data =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        for contract in validate_declarative_contract_document(&data, &path)? {
            if !seen_ids.insert(contract.id.clone()) {
                return Err(format!(
                    "duplicate declarative contract id {:?} in {}",
                    contract.id,
                    path.display()
                ));
            }
            contracts.push(contract);
        }
    }

    contracts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(contracts)
}

fn generate_shellcheck_map_data(mappings: &[(String, u32)]) -> String {
    let mut generated = String::from("pub const RULE_SHELLCHECK_CODES: &[(&str, u32)] = &[\n");
    for (rule_code, sc_number) in mappings {
        generated.push_str(&format!("    ({rule_code:?}, {sc_number}),\n"));
    }
    generated.push_str("];\n");
    generated
}

fn generate_rule_metadata_data(rows: &[RuleMetadataRow]) -> String {
    let mut generated = String::from("pub const RULE_METADATA: &[RuleMetadata] = &[\n");
    for row in rows {
        generated.push_str("    RuleMetadata {\n");
        generated.push_str(&format!("        code: {:?},\n", row.code));
        generated.push_str(&format!(
            "        shellcheck_level: {},\n",
            match row.shellcheck_level {
                Some(ShellCheckLevel::Style) => "Some(ShellCheckLevel::Style)",
                Some(ShellCheckLevel::Info) => "Some(ShellCheckLevel::Info)",
                Some(ShellCheckLevel::Warning) => "Some(ShellCheckLevel::Warning)",
                Some(ShellCheckLevel::Error) => "Some(ShellCheckLevel::Error)",
                None => "None",
            }
        ));
        generated.push_str(&format!("        description: {:?},\n", row.description));
        generated.push_str(&format!("        rationale: {:?},\n", row.rationale));
        generated.push_str(&format!(
            "        fix_description: {},\n",
            match &row.fix_description {
                Some(value) => format!("Some({value:?})"),
                None => "None".to_owned(),
            }
        ));
        generated.push_str("    },\n");
    }
    generated.push_str("];\n");
    generated
}

fn generate_declarative_contract_data(contracts: &[NormalizedDeclarativeContract]) -> String {
    let mut generated = format!(
        "static DECLARATIVE_CONTRACT_DATA: [DeclarativeContractDescriptor; {}] = [\n",
        contracts.len()
    );
    for contract in contracts {
        generated.push_str("    DeclarativeContractDescriptor {\n");
        generated.push_str(&format!("        id: {:?},\n", contract.id));
        generated.push_str(&format!(
            "        groups: {},\n",
            format_string_slice(&contract.groups)
        ));
        generated.push_str(&format!(
            "        label: {},\n",
            match &contract.label {
                Some(label) => format!("Some({label:?})"),
                None => "None".to_owned(),
            }
        ));
        generated.push_str("        activation: ");
        generated.push_str(&format_activation(&contract.activation));
        generated.push_str(",\n");
        generated.push_str(&format!(
            "        files: {},\n",
            format_string_slice(&contract.files)
        ));
        generated.push_str("        effects: DeclarativeEffectsDescriptor {\n");
        generated.push_str(&format!(
            "            reads: {},\n",
            format_string_slice(&contract.effects.reads)
        ));
        generated.push_str(&format!(
            "            consumes_names: {},\n",
            format_string_slice(&contract.effects.consumes_names)
        ));
        generated.push_str(&format!(
            "            consumes_prefixes: {},\n",
            format_string_slice(&contract.effects.consumes_prefixes)
        ));
        generated.push_str(&format!(
            "            consumes_all: {},\n",
            contract.effects.consumes_all
        ));
        generated.push_str(&format!(
            "            provides_variables: {},\n",
            format_string_slice(&contract.effects.provides_variables)
        ));
        generated.push_str(&format!(
            "            provides_functions: {},\n",
            format_string_slice(&contract.effects.provides_functions)
        ));
        generated.push_str("            functions: &[\n");
        for function in &contract.effects.functions {
            generated.push_str("                DeclarativeFunctionDescriptor {\n");
            generated.push_str(&format!("                    name: {:?},\n", function.name));
            generated.push_str(&format!(
                "                    reads: {},\n",
                format_string_slice(&function.reads)
            ));
            generated.push_str(&format!(
                "                    sets: {},\n",
                format_string_slice(&function.sets)
            ));
            generated.push_str("                },\n");
        }
        generated.push_str("            ],\n");
        generated.push_str("        },\n");
        generated.push_str("        compiled_files: std::sync::OnceLock::new(),\n");
        generated.push_str("        file_entry_contract: std::sync::OnceLock::new(),\n");
        generated.push_str("        imported_contract: std::sync::OnceLock::new(),\n");
        generated.push_str("        requesting_file_contract: std::sync::OnceLock::new(),\n");
        generated.push_str("    },\n");
    }
    generated.push_str("];\n");
    generated.push_str("static DECLARATIVE_CONTRACTS: &[DeclarativeContractDescriptor] = &DECLARATIVE_CONTRACT_DATA;\n");
    generated
}

fn format_activation(activation: &NormalizedDeclarativeActivation) -> String {
    match activation {
        NormalizedDeclarativeActivation::Always => {
            "DeclarativeActivationDescriptor::Always".to_owned()
        }
        NormalizedDeclarativeActivation::ZshPlugin { framework, plugin } => format!(
            "DeclarativeActivationDescriptor::ZshPlugin {{ framework: {framework:?}, plugin: {plugin:?} }}"
        ),
        NormalizedDeclarativeActivation::ZshTheme { framework, theme } => format!(
            "DeclarativeActivationDescriptor::ZshTheme {{ framework: {framework:?}, theme: {theme:?} }}"
        ),
    }
}

fn format_string_slice(values: &[String]) -> String {
    if values.is_empty() {
        return "&[]".to_owned();
    }

    let mut rendered = String::from("&[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        write!(&mut rendered, "{value:?}").expect("write to string");
    }
    rendered.push(']');
    rendered
}

#[cfg(not(test))]
fn main() {
    println!("cargo:rustc-check-cfg=cfg(shuck_profiling)");
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=OUT_DIR");
    let profile = env::var("PROFILE").unwrap_or_default();
    let out_dir = env::var_os("OUT_DIR").map(PathBuf::from);
    let out_dir_uses_profiling_profile = out_dir.as_ref().is_some_and(|path| {
        let mut previous = None;
        for component in path.components() {
            if component.as_os_str() == "build" {
                return previous == Some("profiling");
            }
            previous = component.as_os_str().to_str();
        }
        false
    });
    if profile == "profiling" || out_dir_uses_profiling_profile {
        println!("cargo:rustc-cfg=shuck_profiling");
    }

    let manifest_dir = PathBuf::from(match env::var("CARGO_MANIFEST_DIR") {
        Ok(value) => value,
        Err(err) => panic!("CARGO_MANIFEST_DIR: {err}"),
    });
    let docs_dir = manifest_dir.join("rules");
    println!("cargo:rerun-if-changed={}", docs_dir.display());

    let contracts_dir = manifest_dir.join("contracts");
    println!("cargo:rerun-if-changed={}", contracts_dir.display());

    let (mappings, metadata_rows) =
        load_rule_metadata(&docs_dir).unwrap_or_else(|err| panic!("{err}"));
    for path in collect_yaml_files(&docs_dir).unwrap_or_else(|err| panic!("{err}")) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for path in collect_yaml_files(&contracts_dir).unwrap_or_else(|err| panic!("{err}")) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let contracts =
        load_declarative_contracts(&contracts_dir).unwrap_or_else(|err| panic!("{err}"));

    let out_dir = PathBuf::from(match env::var("OUT_DIR") {
        Ok(value) => value,
        Err(err) => panic!("OUT_DIR: {err}"),
    });

    let shellcheck_out_path = out_dir.join("shellcheck_map_data.rs");
    fs::write(
        &shellcheck_out_path,
        generate_shellcheck_map_data(&mappings),
    )
    .unwrap_or_else(|err| panic!("write {}: {err}", shellcheck_out_path.display()));

    let metadata_out_path = out_dir.join("rule_metadata_data.rs");
    fs::write(
        &metadata_out_path,
        generate_rule_metadata_data(&metadata_rows),
    )
    .unwrap_or_else(|err| panic!("write {}: {err}", metadata_out_path.display()));

    let contracts_out_path = out_dir.join("ambient_contracts_data.rs");
    fs::write(
        &contracts_out_path,
        generate_declarative_contract_data(&contracts),
    )
    .unwrap_or_else(|err| panic!("write {}: {err}", contracts_out_path.display()));
}
