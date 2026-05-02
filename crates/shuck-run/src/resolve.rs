use anyhow::Result;

use crate::managed::install_with_environment;
use crate::metadata::read_script_info;
use crate::system::resolve_system;
use crate::types::parse_shell_name;
use crate::{
    Environment, ResolveOptions, ResolvedInterpreter, RunConfig, Shell, VersionConstraint,
};

pub(crate) fn resolve_with_environment(
    environment: &Environment,
    options: ResolveOptions<'_>,
) -> Result<ResolvedInterpreter> {
    let script_info = options
        .script
        .map(read_script_info)
        .transpose()?
        .unwrap_or_default();

    let config_shell = shell_from_config(options.config)?;
    let metadata_shell = script_info.metadata.as_ref().map(|metadata| metadata.shell);
    let inferred_shell = script_info.inferred_shell;
    let shell = options
        .shell
        .or(metadata_shell)
        .or(config_shell)
        .or(inferred_shell)
        .unwrap_or(Shell::Bash);
    let defaulted_shell = options.shell.is_none()
        && metadata_shell.is_none()
        && config_shell.is_none()
        && inferred_shell.is_none();

    let metadata_version = if metadata_shell == Some(shell) {
        script_info
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.version.clone())
    } else {
        None
    };
    let config_version = config_version_for_shell(options.config, shell)?;
    let defaulted_version =
        options.version.is_none() && metadata_version.is_none() && config_version.is_none();
    let version = if let Some(version) = options.version {
        version
    } else if let Some(version) = metadata_version.clone() {
        version
    } else if let Some(version) = config_version.clone() {
        version
    } else {
        VersionConstraint::Latest
    };

    if options.system || (defaulted_shell && defaulted_version) {
        resolve_system(shell, &version)
    } else {
        install_with_environment(
            environment,
            shell,
            &version,
            options.verbose,
            options.refresh_registry,
        )
    }
}

fn shell_from_config(config: Option<&RunConfig>) -> Result<Option<Shell>> {
    let Some(config) = config else {
        return Ok(None);
    };
    config.shell.as_deref().map(parse_shell_name).transpose()
}

fn config_version_for_shell(
    config: Option<&RunConfig>,
    shell: Shell,
) -> Result<Option<VersionConstraint>> {
    let Some(config) = config else {
        return Ok(None);
    };

    if let Some(raw) = config.shells.get(shell.as_str()) {
        return Ok(Some(VersionConstraint::parse(raw)?));
    }

    config
        .shell_version
        .as_deref()
        .map(VersionConstraint::parse)
        .transpose()
}
