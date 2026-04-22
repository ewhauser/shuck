mod build_script {
    #![allow(dead_code)]

    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs"));

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
}
