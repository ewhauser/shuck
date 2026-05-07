mod build_script {
    #![allow(dead_code)]

    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs"));

    use tempfile::tempdir;

    #[test]
    fn parse_shellcheck_code_value_skips_nullable_forms() {
        assert_eq!(parse_shellcheck_code_value(""), Ok(None));
        assert_eq!(parse_shellcheck_code_value("null"), Ok(None));
        assert_eq!(parse_shellcheck_code_value("NULL"), Ok(None));
        assert_eq!(parse_shellcheck_code_value("~"), Ok(None));
        assert_eq!(parse_shellcheck_code_value("\"null\""), Ok(None));
    }

    #[test]
    fn parse_shellcheck_code_value_accepts_quoted_codes() {
        assert_eq!(parse_shellcheck_code_value("SC2034"), Ok(Some(2034)));
        assert_eq!(parse_shellcheck_code_value("\"SC2034\""), Ok(Some(2034)));
        assert_eq!(parse_shellcheck_code_value("'sc2034'"), Ok(Some(2034)));
    }

    #[test]
    fn parse_shellcheck_level_value_accepts_quoted_levels() {
        assert_eq!(
            parse_shellcheck_level_value("warning"),
            Ok(Some(ShellCheckLevel::Warning))
        );
        assert_eq!(
            parse_shellcheck_level_value("\"info\""),
            Ok(Some(ShellCheckLevel::Info))
        );
        assert_eq!(
            parse_shellcheck_level_value("'STYLE'"),
            Ok(Some(ShellCheckLevel::Style))
        );
    }

    #[test]
    fn parse_rule_metadata_accepts_quoted_new_code() {
        let yaml = r#"
new_code: "C001"
shellcheck_code: SC2034
shellcheck_level: warning
description: Example description
rationale: Example rationale
"#;

        let (metadata, shellcheck_code, shellcheck_level) = parse_rule_metadata(yaml).unwrap();
        assert_eq!(metadata.new_code, "C001");
        assert_eq!(metadata.description, "Example description");
        assert_eq!(metadata.rationale, "Example rationale");
        assert_eq!(metadata.fix_description, None);
        assert_eq!(shellcheck_code, Some(2034));
        assert_eq!(shellcheck_level, Some(ShellCheckLevel::Warning));
    }

    #[test]
    fn parse_rule_metadata_accepts_inline_shellcheck_comments() {
        let yaml = r#"
new_code: C001
shellcheck_code: SC2034 # compatibility code
shellcheck_level: warning
description: Example description
rationale: Example rationale
"#;

        let (metadata, shellcheck_code, shellcheck_level) = parse_rule_metadata(yaml).unwrap();
        assert_eq!(metadata.new_code, "C001");
        assert_eq!(metadata.description, "Example description");
        assert_eq!(metadata.rationale, "Example rationale");
        assert_eq!(shellcheck_code, Some(2034));
        assert_eq!(shellcheck_level, Some(ShellCheckLevel::Warning));
    }

    #[test]
    fn parse_rule_metadata_treats_null_shellcheck_code_as_unmapped() {
        let yaml = r#"
new_code: C001
shellcheck_code: null
shellcheck_level: null
description: Example description
rationale: Example rationale
"#;

        let (metadata, shellcheck_code, shellcheck_level) = parse_rule_metadata(yaml).unwrap();
        assert_eq!(metadata.new_code, "C001");
        assert_eq!(metadata.description, "Example description");
        assert_eq!(metadata.rationale, "Example rationale");
        assert_eq!(shellcheck_code, None);
        assert_eq!(shellcheck_level, None);
    }

    #[test]
    fn parse_rule_metadata_requires_shellcheck_level_for_mapped_rules() {
        let yaml = r#"
new_code: C001
shellcheck_code: SC2034
description: Example description
rationale: Example rationale
"#;

        let error = parse_rule_metadata(yaml).unwrap_err();
        assert_eq!(
            error,
            "shellcheck_level must be set when shellcheck_code is set"
        );
    }

    #[test]
    fn parse_rule_metadata_keeps_shellcheck_level_without_code() {
        let yaml = r#"
new_code: C001
shellcheck_code: null
shellcheck_level: info
description: Example description
rationale: Example rationale
"#;

        let (_, shellcheck_code, shellcheck_level) = parse_rule_metadata(yaml).unwrap();
        assert_eq!(shellcheck_code, None);
        assert_eq!(shellcheck_level, Some(ShellCheckLevel::Info));
    }

    #[test]
    fn parse_declarative_contract_document_accepts_tmux_contract() {
        let yaml = r#"
version: 1
contracts:
  - id: zsh/oh-my-zsh/plugin/tmux
    groups:
      - zsh
      - zsh/oh-my-zsh
      - zsh/oh-my-zsh/plugin
    when:
      type: zsh_plugin
      framework: oh-my-zsh
      plugin: tmux
    effects:
      consumes:
        prefixes:
          - ZSH_TMUX_
"#;

        let document = parse_declarative_contract_document(yaml).unwrap();
        assert_eq!(document.version, 1);
        assert_eq!(document.contracts.len(), 1);
        assert_eq!(document.contracts[0].id, "zsh/oh-my-zsh/plugin/tmux");
    }

    #[test]
    fn validate_declarative_contract_document_rejects_empty_effects() {
        let yaml = r#"
version: 1
contracts:
  - id: runtime/example
    groups:
      - runtime
    when:
      type: always
    effects: {}
"#;

        let error =
            validate_declarative_contract_document(yaml, std::path::Path::new("/tmp/example.yaml"))
                .unwrap_err();
        assert!(error.contains("must define at least one effect"));
    }

    #[test]
    fn load_declarative_contracts_rejects_duplicate_ids_across_files() {
        let tempdir = tempdir().unwrap();
        let first = tempdir.path().join("first.yaml");
        let second = tempdir.path().join("second.yaml");
        let yaml = r#"
version: 1
contracts:
  - id: runtime/example
    groups:
      - runtime
    when:
      type: always
    effects:
      provides:
        variables:
          - GITHUB_ENV
"#;
        std::fs::write(&first, yaml).unwrap();
        std::fs::write(&second, yaml).unwrap();

        let error = load_declarative_contracts(tempdir.path()).unwrap_err();
        assert!(error.contains("duplicate declarative contract id"));
    }

    #[test]
    fn load_declarative_contracts_rejects_rust_built_in_id_collisions() {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("collision.yaml");
        let yaml = r#"
version: 1
contracts:
  - id: zsh/runtime
    groups:
      - zsh
    when:
      type: always
    effects:
      provides:
        variables:
          - GITHUB_ENV
"#;
        std::fs::write(&path, yaml).unwrap();

        let error = load_declarative_contracts(tempdir.path()).unwrap_err();
        assert!(error.contains("conflicts with existing Rust built-in contract id"));
    }

    #[test]
    fn generate_declarative_contract_data_renders_runtime_descriptors() {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("tmux.yaml");
        std::fs::write(
            &path,
            r#"
version: 1
contracts:
  - id: zsh/oh-my-zsh/plugin/tmux
    groups:
      - zsh
      - zsh/oh-my-zsh
      - zsh/oh-my-zsh/plugin
    label: Oh My Zsh tmux plugin
    when:
      type: zsh_plugin
      framework: oh-my-zsh
      plugin: tmux
    effects:
      consumes:
        prefixes:
          - ZSH_TMUX_
"#,
        )
        .unwrap();

        let contracts = load_declarative_contracts(tempdir.path()).unwrap();
        let generated = generate_declarative_contract_data(&contracts);

        assert!(generated.contains("static DECLARATIVE_CONTRACT_DATA"));
        assert!(generated.contains("static DECLARATIVE_CONTRACTS"));
        assert!(generated.contains("\"zsh/oh-my-zsh/plugin/tmux\""));
        assert!(generated.contains("std::sync::OnceLock::new()"));
        assert!(generated.contains("\"ZSH_TMUX_\""));
    }
}
