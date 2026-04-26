use std::path::Path;

use shuck_extract::{EmbeddedScript, ExpressionTaint, ExtractedDialect, extract_all};

const WORKFLOW_EDGE_CASES: &str = include_str!("fixtures/github-actions-edge-cases.yml");
const COMPOSITE_ACTION: &str = include_str!("fixtures/github-actions-composite-action.yml");

#[test]
fn extracts_workflow_edge_case_fixture() {
    let scripts = extract_all(
        Path::new(".github/workflows/edge-cases.yml"),
        WORKFLOW_EDGE_CASES,
    )
    .unwrap();

    assert_eq!(scripts.len(), 13);

    let missing_unix = script(&scripts, "jobs.missing-unix.steps[0].run");
    assert_eq!(missing_unix.dialect, ExtractedDialect::Bash);
    assert!(missing_unix.implicit_flags.errexit);
    assert!(missing_unix.implicit_flags.pipefail);
    assert_eq!(
        missing_unix.implicit_flags.template.as_deref(),
        Some("bash --noprofile --norc -eo pipefail {0}")
    );
    assert_eq!(missing_unix.source, "echo \"$WORKFLOW_VALUE\"\n");

    let defaulted = script(&scripts, "jobs.defaulted.steps[0].run");
    assert_eq!(defaulted.dialect, ExtractedDialect::Sh);
    assert!(defaulted.implicit_flags.errexit);
    assert!(!defaulted.implicit_flags.pipefail);
    assert_eq!(
        defaulted.implicit_flags.template.as_deref(),
        Some("sh -e {0}")
    );

    let bash = script(&scripts, "jobs.explicit-shells.steps[0].run");
    assert_eq!(bash.dialect, ExtractedDialect::Bash);
    assert_eq!(bash.source, "echo \"${_SHUCK_GHA_1}\"\n");
    assert_eq!(bash.placeholders[0].expression, "github.sha");
    assert_eq!(bash.placeholders[0].taint, ExpressionTaint::Trusted);

    let sh = script(&scripts, "jobs.explicit-shells.steps[1].run");
    assert_eq!(sh.dialect, ExtractedDialect::Sh);
    assert_eq!(sh.source, "echo \"$STEP_VALUE\"\n");

    let custom = script(&scripts, "jobs.explicit-shells.steps[2].run");
    assert_eq!(custom.dialect, ExtractedDialect::Bash);
    assert!(custom.implicit_flags.errexit);
    assert!(custom.implicit_flags.pipefail);
    assert_eq!(
        custom.implicit_flags.template.as_deref(),
        Some("bash --noprofile --norc -e -o pipefail {0}")
    );
    assert_eq!(custom.source, "printf '%s\\n' \"${_SHUCK_GHA_1}\"\n");
    assert_eq!(
        custom.placeholders[0].expression,
        "github.event.pull_request.title"
    );
    assert_eq!(
        custom.placeholders[0].taint,
        ExpressionTaint::UserControlled
    );

    let pwsh = script(&scripts, "jobs.explicit-shells.steps[3].run");
    assert_eq!(pwsh.dialect, ExtractedDialect::Unsupported);

    let windows_default = script(&scripts, "jobs.windows.steps[0].run");
    assert_eq!(windows_default.dialect, ExtractedDialect::Unsupported);

    let windows_shell = script(&scripts, "jobs.windows.steps[1].run");
    assert_eq!(windows_shell.dialect, ExtractedDialect::Unsupported);

    let windows_bash = script(&scripts, "jobs.windows.steps[2].run");
    assert_eq!(windows_bash.dialect, ExtractedDialect::Bash);

    let anchored = script(&scripts, "jobs.anchors-and-env.steps[0].run");
    assert_eq!(anchored.dialect, ExtractedDialect::Bash);
    assert_eq!(anchored.source, "echo \"$WORKFLOW_VALUE\"\n");

    let env_alias = script(&scripts, "jobs.anchors-and-env.steps[1].run");
    assert_eq!(env_alias.dialect, ExtractedDialect::Bash);
    assert_eq!(env_alias.source, "echo \"${_SHUCK_GHA_1}\"\n");
    assert_eq!(env_alias.placeholders[0].expression, "env.WORKFLOW_VALUE");
    assert_eq!(env_alias.placeholders[0].taint, ExpressionTaint::Trusted);

    let anchored_block = script(&scripts, "jobs.anchors-and-env.steps[2].run");
    assert_eq!(anchored_block.dialect, ExtractedDialect::Bash);
    assert!(anchored_block.source.contains("*prod) echo prod ;;"));

    let flow_style = script(&scripts, "jobs.flow-style.steps[0].run");
    assert_eq!(flow_style.dialect, ExtractedDialect::Sh);
    assert_eq!(flow_style.source, "echo flow");
    assert_eq!(flow_style.host_start_column, 41);
}

#[test]
fn extracts_composite_action_edge_case_fixture() {
    let scripts = extract_all(Path::new("action.yml"), COMPOSITE_ACTION).unwrap();

    assert_eq!(scripts.len(), 3);

    let bash = script(&scripts, "runs.steps[0].run");
    assert_eq!(bash.dialect, ExtractedDialect::Bash);
    assert_eq!(
        bash.source,
        "echo \"$COMPOSITE_VALUE\"\necho \"${_SHUCK_GHA_1}\"\n"
    );
    assert_eq!(bash.placeholders[0].expression, "github.sha");
    assert_eq!(bash.placeholders[0].taint, ExpressionTaint::Trusted);

    let sh = script(&scripts, "runs.steps[1].run");
    assert_eq!(sh.dialect, ExtractedDialect::Sh);
    assert_eq!(sh.source, "echo \"${_SHUCK_GHA_1}\"");
    assert_eq!(sh.placeholders[0].expression, "inputs.message");
    assert_eq!(sh.placeholders[0].taint, ExpressionTaint::Unknown);

    let unsupported = script(&scripts, "runs.steps[2].run");
    assert_eq!(unsupported.dialect, ExtractedDialect::Unsupported);
}

fn script<'a>(scripts: &'a [EmbeddedScript], label: &str) -> &'a EmbeddedScript {
    scripts
        .iter()
        .find(|script| script.label == label)
        .unwrap_or_else(|| panic!("missing extracted script `{label}`"))
}
