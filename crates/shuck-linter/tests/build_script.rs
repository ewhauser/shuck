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
    fn parse_rule_metadata_accepts_quoted_new_code() {
        let yaml = r#"
new_code: "C001"
shellcheck_code: SC2034
"#;

        assert_eq!(
            parse_rule_metadata(yaml),
            Ok(("C001".to_owned(), Some(2034)))
        );
    }

    #[test]
    fn parse_rule_metadata_accepts_inline_shellcheck_comments() {
        let yaml = r#"
new_code: C001
shellcheck_code: SC2034 # compatibility code
"#;

        assert_eq!(
            parse_rule_metadata(yaml),
            Ok(("C001".to_owned(), Some(2034)))
        );
    }

    #[test]
    fn parse_rule_metadata_treats_null_shellcheck_code_as_unmapped() {
        let yaml = r#"
new_code: C001
shellcheck_code: null
"#;

        assert_eq!(parse_rule_metadata(yaml), Ok(("C001".to_owned(), None)));
    }
}
