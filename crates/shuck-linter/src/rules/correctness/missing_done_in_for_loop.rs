use crate::{FixAvailability, Rule, Violation};

pub struct MissingDoneInForLoop;

impl Violation for MissingDoneInForLoop {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::MissingDoneInForLoop
    }

    fn message(&self) -> String {
        "this `for` loop is missing a closing `done`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append a closing `done`".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn applies_unsafe_fix_by_appending_done() {
        let source = "\
#!/bin/sh
for x in a; do
  echo \"$x\"";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MissingDoneInForLoop),
            Applicability::Unsafe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
for x in a; do
  echo \"$x\"
done
"
        );
        assert_eq!(result.fixes_applied, 1);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_missing_done_unchanged() {
        let source = "\
#!/bin/sh
for x in a; do
  echo \"$x\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MissingDoneInForLoop),
            Applicability::Safe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C142.sh").as_path(),
            &LinterSettings::for_rule(Rule::MissingDoneInForLoop),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C142_fix_C142.sh", result);
        Ok(())
    }
}
