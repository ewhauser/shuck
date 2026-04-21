use std::env;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RuleMetadata {
    new_code: String,
    shellcheck_code: Option<String>,
    description: String,
    rationale: String,
    fix_description: Option<String>,
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

fn parse_rule_metadata(data: &str) -> Result<(RuleMetadata, Option<u32>), String> {
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

    Ok((metadata, shellcheck_code))
}

fn main() {
    let manifest_dir = PathBuf::from(match env::var("CARGO_MANIFEST_DIR") {
        Ok(value) => value,
        Err(err) => panic!("CARGO_MANIFEST_DIR: {err}"),
    });
    // `rules` is a symlink to `../../docs/rules` in the workspace; cargo
    // follows the symlink when packaging so the YAML ships with the crate.
    let docs_dir = manifest_dir.join("rules");
    println!("cargo:rerun-if-changed={}", docs_dir.display());

    let mut entries = fs::read_dir(&docs_dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", docs_dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect::<Vec<_>>();
    entries.sort();

    let mut mappings = Vec::new();
    let mut metadata_rows = Vec::new();

    for path in entries {
        println!("cargo:rerun-if-changed={}", path.display());
        let data = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));

        let (metadata, shellcheck_code) =
            parse_rule_metadata(&data).unwrap_or_else(|err| panic!("{err} in {}", path.display()));
        let rule_code = metadata.new_code.trim().to_owned();
        metadata_rows.push((
            rule_code.clone(),
            metadata.description,
            metadata.rationale,
            metadata.fix_description,
        ));

        let Some(shellcheck_code) = shellcheck_code else {
            continue;
        };
        mappings.push((rule_code, shellcheck_code));
    }

    let mut generated = String::from("pub const RULE_SHELLCHECK_CODES: &[(&str, u32)] = &[\n");
    for (rule_code, sc_number) in mappings {
        generated.push_str(&format!("    (\"{rule_code}\", {sc_number}),\n"));
    }
    generated.push_str("];\n");

    let out_path = PathBuf::from(match env::var("OUT_DIR") {
        Ok(value) => value,
        Err(err) => panic!("OUT_DIR: {err}"),
    })
    .join("shellcheck_map_data.rs");
    fs::write(&out_path, generated)
        .unwrap_or_else(|err| panic!("write {}: {err}", out_path.display()));

    let mut metadata_generated = String::from("pub const RULE_METADATA: &[RuleMetadata] = &[\n");
    for (rule_code, description, rationale, fix_description) in metadata_rows {
        metadata_generated.push_str("    RuleMetadata {\n");
        metadata_generated.push_str(&format!("        code: {:?},\n", rule_code));
        metadata_generated.push_str(&format!("        description: {:?},\n", description));
        metadata_generated.push_str(&format!("        rationale: {:?},\n", rationale));
        metadata_generated.push_str(&format!(
            "        fix_description: {},\n",
            match fix_description {
                Some(value) => format!("Some({value:?})"),
                None => "None".to_owned(),
            }
        ));
        metadata_generated.push_str("    },\n");
    }
    metadata_generated.push_str("];\n");

    let metadata_out_path = PathBuf::from(match env::var("OUT_DIR") {
        Ok(value) => value,
        Err(err) => panic!("OUT_DIR: {err}"),
    })
    .join("rule_metadata_data.rs");
    fs::write(&metadata_out_path, metadata_generated)
        .unwrap_or_else(|err| panic!("write {}: {err}", metadata_out_path.display()));
}
