use anyhow::Result;

use crate::ExitStatus;

pub(crate) fn server() -> Result<ExitStatus> {
    shuck_server::run()?;
    Ok(ExitStatus::Success)
}
