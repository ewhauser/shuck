#![warn(missing_docs)]

//! Shell runtime discovery, version resolution, and managed installation.

mod download;
mod environment;
mod managed;
mod metadata;
mod registry;
mod resolve;
mod system;
mod types;

#[cfg(all(test, unix))]
mod tests;

use std::path::Path;

use anyhow::Result;

use environment::Environment;
use managed::install_with_environment;
use registry::{available_shells, load_registry};
use resolve::resolve_with_environment;

pub use types::{
    AvailableShell, ResolutionSource, ResolveOptions, ResolvedInterpreter, RunConfig, Shell,
    Version, VersionConstraint, VersionPredicate,
};

/// Resolve a shell interpreter using direct arguments plus optional config.
pub fn resolve(
    shell: Option<Shell>,
    version: Option<VersionConstraint>,
    system: bool,
    script: Option<&Path>,
    config: Option<&RunConfig>,
) -> Result<ResolvedInterpreter> {
    resolve_with_options(ResolveOptions::new(shell, version, system, script, config))
}

/// Resolve a shell interpreter using a fully constructed options value.
pub fn resolve_with_options(options: ResolveOptions<'_>) -> Result<ResolvedInterpreter> {
    let environment = Environment::from_process()?;
    resolve_with_environment(&environment, options)
}

/// Install a managed shell version if needed, then return its interpreter path.
pub fn install(shell: Shell, version: &VersionConstraint) -> Result<ResolvedInterpreter> {
    install_with_options(shell, version, false, false)
}

/// Install a managed shell version with control over logging and registry refresh.
pub fn install_with_options(
    shell: Shell,
    version: &VersionConstraint,
    verbose: bool,
    refresh_registry: bool,
) -> Result<ResolvedInterpreter> {
    let environment = Environment::from_process()?;
    install_with_environment(&environment, shell, version, verbose, refresh_registry)
}

/// List managed shell versions known to the local or remote registry.
pub fn list_available(shell: Option<Shell>) -> Result<Vec<AvailableShell>> {
    list_available_with_options(shell, false, false)
}

/// List managed shell versions with control over logging and registry refresh.
pub fn list_available_with_options(
    shell: Option<Shell>,
    refresh_registry: bool,
    verbose: bool,
) -> Result<Vec<AvailableShell>> {
    let environment = Environment::from_process()?;
    let registry = load_registry(&environment, refresh_registry, verbose)?;
    Ok(available_shells(&registry, shell))
}
