use std::path::PathBuf;

use anyhow::{Context, Result};

const DEFAULT_REGISTRY_URL: &str = "https://ewhauser.github.io/shuck-shells/";
const SHELLS_DIR_ENV: &str = "SHUCK_SHELLS_DIR";
const REGISTRY_URL_ENV: &str = "SHUCK_RUN_REGISTRY_URL";

#[derive(Debug, Clone)]
pub(crate) struct Environment {
    pub(crate) shells_root: PathBuf,
    pub(crate) registry_url: String,
}

impl Environment {
    pub(crate) fn from_process() -> Result<Self> {
        let shells_root = if let Some(value) = std::env::var_os(SHELLS_DIR_ENV) {
            PathBuf::from(value)
        } else {
            let home = etcetera::home_dir().context("resolve the home directory for shuck")?;
            home.join(".shuck").join("shells")
        };

        let registry_url =
            std::env::var(REGISTRY_URL_ENV).unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_owned());

        Ok(Self {
            shells_root,
            registry_url,
        })
    }
}

pub(crate) fn current_platform() -> Result<String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok(format!("x86_64-linux-{}", linux_runtime_abi()?)),
        ("linux", "aarch64") => Ok(format!("aarch64-linux-{}", linux_runtime_abi()?)),
        ("macos", "x86_64") => Ok("x86_64-darwin".to_owned()),
        ("macos", "aarch64") => Ok("aarch64-darwin".to_owned()),
        (os, arch) => anyhow::bail!("unsupported platform {arch}-{os}"),
    }
}

fn linux_runtime_abi() -> Result<&'static str> {
    if cfg!(target_env = "gnu") {
        Ok("gnu")
    } else if cfg!(target_env = "musl") {
        Ok("musl")
    } else {
        anyhow::bail!("unsupported linux target environment for shuck run")
    }
}
