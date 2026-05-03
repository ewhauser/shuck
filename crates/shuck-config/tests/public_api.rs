use std::fs;

use shuck_config::{
    CONFIG_DIALECT_UNSUPPORTED_ERROR, ConfigArguments, FormatConfig, ShuckConfig,
    SingleConfigArgument, discovered_config_path_for_root, load_project_config,
    resolve_project_root_for_file, resolve_project_root_for_input,
};
use tempfile::tempdir;

#[test]
fn public_api_layers_discovered_config_and_inline_overrides() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\nfunction-next-line = true\n",
    )
    .unwrap();

    let override_config = ShuckConfig {
        format: FormatConfig {
            indent_width: Some(2),
            ..FormatConfig::default()
        },
        ..ShuckConfig::default()
    };

    let config = ConfigArguments::from_cli(
        vec![SingleConfigArgument::SettingsOverride(Box::new(
            override_config,
        ))],
        false,
    )
    .unwrap();

    let loaded = load_project_config(tempdir.path(), &config).unwrap();
    assert_eq!(loaded.format.function_next_line, Some(true));
    assert_eq!(loaded.format.indent_width, Some(2));
}

#[test]
fn public_api_prefers_explicit_config_file_over_discovered_file() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\nfunction-next-line = false\n",
    )
    .unwrap();

    let explicit = tempdir.path().join("override.toml");
    fs::write(&explicit, "[format]\nfunction-next-line = true\n").unwrap();

    let config =
        ConfigArguments::from_cli(vec![SingleConfigArgument::FilePath(explicit)], false).unwrap();

    let loaded = load_project_config(tempdir.path(), &config).unwrap();
    assert_eq!(loaded.format.function_next_line, Some(true));
}

#[test]
fn public_api_resolves_project_roots_and_discovered_config_paths() {
    let tempdir = tempdir().unwrap();
    let nested = tempdir.path().join("nested");
    let file = nested.join("script.sh");

    fs::create_dir_all(&nested).unwrap();
    fs::write(tempdir.path().join(".shuck.toml"), "[format]\n").unwrap();
    fs::write(&file, "#!/bin/sh\necho hi\n").unwrap();

    assert_eq!(
        resolve_project_root_for_input(&nested, true).unwrap(),
        tempdir.path()
    );
    assert_eq!(
        resolve_project_root_for_file(&file, &nested, true).unwrap(),
        tempdir.path()
    );
    assert_eq!(
        discovered_config_path_for_root(tempdir.path()).unwrap(),
        Some(tempdir.path().join(".shuck.toml"))
    );
}

#[test]
fn public_api_rejects_format_dialect_in_config_patch() {
    let config = FormatConfig {
        dialect: Some(toml::Value::String("zsh".to_owned())),
        ..FormatConfig::default()
    };

    let err = config.to_patch().unwrap_err();
    assert_eq!(err.to_string(), CONFIG_DIALECT_UNSUPPORTED_ERROR);
}
