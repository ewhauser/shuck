pub mod conditional_portability;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::DoubleBracketInSh, Path::new("X001.sh"))]
    #[test_case(Rule::TestEqualityOperator, Path::new("X002.sh"))]
    #[test_case(Rule::IfElifBashTest, Path::new("X033.sh"))]
    #[test_case(Rule::ExtendedGlobInTest, Path::new("X034.sh"))]
    #[test_case(Rule::ArraySubscriptTest, Path::new("X040.sh"))]
    #[test_case(Rule::ArraySubscriptCondition, Path::new("X041.sh"))]
    #[test_case(Rule::ExtglobInTest, Path::new("X046.sh"))]
    #[test_case(Rule::GreaterThanInDoubleBracket, Path::new("X058.sh"))]
    #[test_case(Rule::RegexMatchInSh, Path::new("X059.sh"))]
    #[test_case(Rule::VTestInSh, Path::new("X060.sh"))]
    #[test_case(Rule::ATestInSh, Path::new("X061.sh"))]
    #[test_case(Rule::OptionTestInSh, Path::new("X073.sh"))]
    #[test_case(Rule::StickyBitTestInSh, Path::new("X074.sh"))]
    #[test_case(Rule::OwnershipTestInSh, Path::new("X075.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("portability").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}
