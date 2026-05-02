use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::{Result, anyhow};
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use tempfile::NamedTempFile;

use shuck_run::{ResolutionSource, ResolveOptions, RunConfig, Shell, VersionConstraint};

use crate::ExitStatus;
use crate::args::{InstallCommand, ManagedShellArg, RunCommand, ShellCommand};
use crate::config::{
    ConfigArguments, load_project_config, resolve_project_root_for_file,
    resolve_project_root_for_input,
};

pub(crate) fn run(args: RunCommand, config_arguments: &ConfigArguments) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let config = load_runtime_config(config_arguments, &cwd, args.script.as_deref())?;
    let resolved = resolve_interpreter(
        args.shell,
        args.shell_version.as_deref(),
        args.system,
        true,
        runtime_resolution_script(&cwd, args.script.as_deref(), args.command.as_deref())?
            .as_deref(),
        Some(&config),
        args.verbose,
    )?;

    if args.dry_run {
        println!(
            "{} {} ({})",
            resolved.shell,
            resolved.version,
            resolved.path.display()
        );
        return Ok(ExitStatus::Success);
    }

    let mut command = ProcessCommand::new(&resolved.path);
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    apply_runtime_environment(&mut command, &resolved);
    append_shell_launcher_args(&mut command, resolved.shell);
    let temp_script = append_run_mode_args(&mut command, &cwd, resolved.shell, &args)?;

    if temp_script.is_some() {
        return wait_for_process(command);
    }

    exec_or_wait(command)
}

pub(crate) fn install(args: InstallCommand) -> Result<ExitStatus> {
    if args.list {
        let available = shuck_run::list_available_with_options(
            args.shell.map(Into::into),
            args.refresh,
            false,
        )?;

        if let Some(shell) = args.shell {
            let shell = Shell::from(shell);
            let versions = available
                .into_iter()
                .find(|entry| entry.shell == shell)
                .map(|entry| entry.versions)
                .unwrap_or_default();
            for version in versions {
                println!("{version}");
            }
        } else {
            for entry in available {
                for version in entry.versions {
                    println!("{} {}", entry.shell, version);
                }
            }
        }

        return Ok(ExitStatus::Success);
    }

    let shell = args
        .shell
        .map(Into::into)
        .ok_or_else(|| anyhow!("missing shell; expected `shuck install <shell> <version>`"))?;
    let version = parse_version_constraint(args.version.as_deref())?;
    let resolved = shuck_run::install_with_options(shell, &version, false, args.refresh)?;
    println!(
        "{} {} ({})",
        resolved.shell,
        resolved.version,
        resolved.path.display()
    );
    Ok(ExitStatus::Success)
}

pub(crate) fn shell(args: ShellCommand, config_arguments: &ConfigArguments) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let config = load_runtime_config(config_arguments, &cwd, None)?;
    let resolved = resolve_interpreter(
        args.shell,
        args.shell_version.as_deref(),
        args.system,
        false,
        None,
        Some(&config),
        args.verbose,
    )?;

    let mut command = ProcessCommand::new(&resolved.path);
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    apply_runtime_environment(&mut command, &resolved);
    append_shell_launcher_args(&mut command, resolved.shell);
    exec_or_wait(command)
}

fn resolve_interpreter(
    shell: Option<ManagedShellArg>,
    shell_version: Option<&str>,
    system: bool,
    implicit_system_fallback: bool,
    script: Option<&Path>,
    config: Option<&RunConfig>,
    verbose: bool,
) -> Result<shuck_run::ResolvedInterpreter> {
    let version = shell_version.map(VersionConstraint::parse).transpose()?;
    shuck_run::resolve_with_options(ResolveOptions {
        shell: shell.map(Into::into),
        version,
        system,
        implicit_system_fallback,
        script,
        config,
        verbose,
        refresh_registry: false,
    })
}

fn parse_version_constraint(raw: Option<&str>) -> Result<VersionConstraint> {
    let raw = raw.ok_or_else(|| anyhow!("missing version constraint"))?;
    VersionConstraint::parse(raw)
}

fn load_runtime_config(
    config_arguments: &ConfigArguments,
    cwd: &Path,
    script: Option<&Path>,
) -> Result<RunConfig> {
    let project_root = match script.filter(|path| !is_stdin_script(Some(path))) {
        Some(path) => {
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            resolve_project_root_for_file(&absolute, cwd, config_arguments.use_config_roots())?
        }
        None => resolve_project_root_for_input(cwd, config_arguments.use_config_roots())?,
    };

    Ok(load_project_config(&project_root, config_arguments)?.run)
}

fn runtime_resolution_script(
    cwd: &Path,
    script: Option<&Path>,
    command: Option<&str>,
) -> Result<Option<PathBuf>> {
    if command.is_some() || is_stdin_script(script) {
        return Ok(None);
    }

    script
        .map(|path| {
            Ok(if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            })
        })
        .transpose()
}

fn append_run_mode_args(
    command: &mut ProcessCommand,
    cwd: &Path,
    shell: Shell,
    args: &RunCommand,
) -> Result<Option<NamedTempFile>> {
    if shell == Shell::Bashkit {
        return append_bashkit_run_mode_args(command, cwd, args);
    }

    append_posix_run_mode_args(command, cwd, args)?;
    Ok(None)
}

fn append_posix_run_mode_args(
    command: &mut ProcessCommand,
    cwd: &Path,
    args: &RunCommand,
) -> Result<()> {
    if let Some(raw_command) = args.command.as_deref() {
        command.arg("-c");
        command.arg(raw_command);
        command.arg("shuck-run");
        command.args(&args.script_args);
        return Ok(());
    }

    if is_stdin_script(args.script.as_deref()) {
        command.arg("-s");
        if !args.script_args.is_empty() {
            command.arg("--");
            command.args(&args.script_args);
        }
        return Ok(());
    }

    let script = args
        .script
        .as_deref()
        .ok_or_else(|| anyhow!("missing script path"))?;
    let script = if script.is_absolute() {
        script.to_path_buf()
    } else {
        cwd.join(script)
    };
    command.arg(script);
    command.args(&args.script_args);
    Ok(())
}

fn append_bashkit_run_mode_args(
    command: &mut ProcessCommand,
    cwd: &Path,
    args: &RunCommand,
) -> Result<Option<NamedTempFile>> {
    if let Some(raw_command) = args.command.as_deref() {
        let temp_script = write_bashkit_command_script(raw_command)?;
        command.arg(temp_script.path());
        command.args(&args.script_args);
        return Ok(Some(temp_script));
    }

    if is_stdin_script(args.script.as_deref()) {
        let temp_script = write_bashkit_stdin_script()?;
        command.arg(temp_script.path());
        command.args(&args.script_args);
        return Ok(Some(temp_script));
    }

    let script = args
        .script
        .as_deref()
        .ok_or_else(|| anyhow!("missing script path"))?;
    let script = if script.is_absolute() {
        script.to_path_buf()
    } else {
        cwd.join(script)
    };

    command.arg(script);
    command.args(&args.script_args);
    Ok(None)
}

fn write_bashkit_command_script(source: &str) -> Result<NamedTempFile> {
    let mut temp_script = tempfile::Builder::new()
        .prefix("shuck-bashkit-")
        .suffix(".sh")
        .tempfile()?;
    temp_script.write_all(source.as_bytes())?;
    if !source.ends_with('\n') {
        temp_script.write_all(b"\n")?;
    }
    temp_script.flush()?;
    Ok(temp_script)
}

fn write_bashkit_stdin_script() -> Result<NamedTempFile> {
    let mut temp_script = tempfile::Builder::new()
        .prefix("shuck-bashkit-")
        .suffix(".sh")
        .tempfile()?;
    std::io::copy(&mut std::io::stdin().lock(), &mut temp_script)?;
    temp_script.flush()?;
    Ok(temp_script)
}

fn append_shell_launcher_args(command: &mut ProcessCommand, shell: Shell) {
    if matches!(shell, Shell::Busybox) {
        command.arg("sh");
    }
}
fn apply_runtime_environment(
    command: &mut ProcessCommand,
    resolved: &shuck_run::ResolvedInterpreter,
) {
    command.env("SHUCK_SHELL", resolved.shell.as_str());
    command.env("SHUCK_SHELL_VERSION", resolved.version.as_str());
    command.env("SHUCK_SHELL_PATH", &resolved.path);

    if matches!(resolved.source, ResolutionSource::Managed)
        && !matches!(resolved.shell, Shell::Busybox)
    {
        command.env("SHELL", &resolved.path);
    }
}

fn is_stdin_script(script: Option<&Path>) -> bool {
    script.is_none() || matches!(script, Some(path) if path == Path::new("-"))
}

#[cfg(unix)]
fn exec_or_wait(mut command: ProcessCommand) -> Result<ExitStatus> {
    let err = command.exec();
    Err(anyhow!(err))
}

fn wait_for_process(mut command: ProcessCommand) -> Result<ExitStatus> {
    let status = command.status()?;
    Ok(exit_status_from_process(status))
}

#[cfg(not(unix))]
fn exec_or_wait(mut command: ProcessCommand) -> Result<ExitStatus> {
    wait_for_process(command)
}

#[cfg(test)]
pub(crate) fn exec_or_wait_for_test(command: ProcessCommand) -> Result<ExitStatus> {
    wait_for_process(command)
}

fn exit_status_from_process(status: std::process::ExitStatus) -> ExitStatus {
    #[cfg(unix)]
    if let Some(signal) = status.signal() {
        return ExitStatus::Code(128 + signal);
    }

    match status.code() {
        Some(0) => ExitStatus::Success,
        Some(code) => ExitStatus::Code(code),
        None => ExitStatus::Failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};

    #[test]
    fn stdin_mode_detection_handles_dash_and_empty_script() {
        assert!(is_stdin_script(None));
        assert!(is_stdin_script(Some(Path::new("-"))));
        assert!(!is_stdin_script(Some(Path::new("deploy.sh"))));
    }

    #[test]
    fn wait_mapping_preserves_child_exit_code() {
        let command = if cfg!(windows) {
            let mut command = ProcessCommand::new("cmd");
            command.args(["/C", "exit 7"]);
            command
        } else {
            let mut command = ProcessCommand::new("sh");
            command.args(["-c", "exit 7"]);
            command
        };
        let status = exec_or_wait_for_test(command).unwrap();
        assert_eq!(status, ExitStatus::Code(7));
    }
    #[cfg(unix)]
    #[test]
    fn wait_mapping_preserves_signal_exit_code() {
        let mut command = ProcessCommand::new("sh");
        command.args(["-c", "kill -INT $$"]);
        let status = exec_or_wait_for_test(command).unwrap();
        assert_eq!(status, ExitStatus::Code(130));
    }

    #[test]
    fn busybox_run_command_invokes_sh_applet() {
        let mut command = ProcessCommand::new("busybox");
        append_shell_launcher_args(&mut command, Shell::Busybox);

        let args = RunCommand {
            shell: None,
            shell_version: None,
            system: false,
            dry_run: false,
            verbose: false,
            command: Some("echo hi".to_owned()),
            script: None,
            script_args: vec![OsString::from("one"), OsString::from("two")],
        };
        let temp_script =
            append_run_mode_args(&mut command, Path::new("/tmp"), Shell::Busybox, &args).unwrap();
        assert!(temp_script.is_none());

        let collected = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            collected,
            vec!["sh", "-c", "echo hi", "shuck-run", "one", "two"]
        );
    }

    #[test]
    fn managed_busybox_does_not_override_shell_env() {
        let mut command = ProcessCommand::new("busybox");
        let resolved = shuck_run::ResolvedInterpreter {
            shell: Shell::Busybox,
            version: shuck_run::Version::parse("1.36.1").unwrap(),
            path: PathBuf::from("/tmp/busybox"),
            source: ResolutionSource::Managed,
        };

        apply_runtime_environment(&mut command, &resolved);

        let shell = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("SHELL"))
            .and_then(|(_, value)| value)
            .map(|value| value.to_os_string());
        assert!(shell.is_none());
    }
}
