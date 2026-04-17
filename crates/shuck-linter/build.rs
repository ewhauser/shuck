use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let docs_dir = manifest_dir.join("../../docs/rules");
    println!("cargo:rerun-if-changed={}", docs_dir.display());

    let mut entries = fs::read_dir(&docs_dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", docs_dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect::<Vec<_>>();
    entries.sort();

    let mut mappings = Vec::new();

    for path in entries {
        println!("cargo:rerun-if-changed={}", path.display());
        let data = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));

        let rule_code = data
            .lines()
            .find_map(|line| {
                line.strip_prefix("new_code:")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| panic!("missing new_code in {}", path.display()));

        let shellcheck_code = data.lines().find_map(|line| {
            line.strip_prefix("shellcheck_code:")
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });

        let Some(shellcheck_code) = shellcheck_code else {
            continue;
        };
        let sc_number = shellcheck_code
            .strip_prefix("SC")
            .unwrap_or_else(|| panic!("invalid shellcheck_code in {}", path.display()))
            .parse::<u32>()
            .unwrap_or_else(|err| panic!("parse shellcheck_code in {}: {err}", path.display()));

        mappings.push((rule_code.to_owned(), sc_number));
    }

    let mut generated = String::from("pub const RULE_SHELLCHECK_CODES: &[(&str, u32)] = &[\n");
    for (rule_code, sc_number) in mappings {
        generated.push_str(&format!("    (\"{rule_code}\", {sc_number}),\n"));
    }
    generated.push_str("];\n");

    let out_path =
        PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("shellcheck_map_data.rs");
    fs::write(&out_path, generated)
        .unwrap_or_else(|err| panic!("write {}: {err}", out_path.display()));
}
