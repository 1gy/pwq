use crate::error::{Error, Result};
use crate::io::{Backend, Io};
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct ProcessBackend {
    child: Child,
    exit_code: Option<i32>,
}

impl Backend for ProcessBackend {
    fn close(&mut self) {
        let _ = self.child.kill();
        if let Ok(status) = self.child.wait() {
            self.exit_code = status.code();
        }
    }

    fn wait(&mut self) -> Result<Option<i32>> {
        if let Some(code) = self.exit_code {
            return Ok(Some(code));
        }
        let status = self.child.wait().map_err(Error::Io)?;
        self.exit_code = status.code();
        Ok(self.exit_code)
    }

    // Linux pipes have no per-fd read timeout; only the no-op clear works.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        match timeout {
            None => Ok(()),
            Some(_) => Err(Error::Unsupported(
                "read timeout on process backend (Linux pipes have no per-fd timeout; \
                 use a remote backend)",
            )),
        }
    }
}

pub fn process<P: AsRef<Path>>(path: P) -> Result<Io> {
    process_with_args(path, std::iter::empty::<&OsStr>())
}

pub fn process_with_args<P, I, S>(path: P, args: I) -> Result<Io>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(path.as_ref())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Other("child stdin not piped".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Other("child stdout not piped".into()))?;
    Ok(Io::new(
        Box::new(stdout),
        Box::new(stdin),
        Box::new(ProcessBackend {
            child,
            exit_code: None,
        }),
    ))
}
