//! Child-process seams. The OpenRGB server and the daemon are started through these traits
//! so tests use fakes. Nothing in the test suite launches a real process.

use std::io;
use std::path::Path;

pub trait ChildProcess: Send {
    /// `Some(exit_code)` once the process has exited, `None` while it runs.
    fn try_wait(&mut self) -> io::Result<Option<i32>>;
    fn kill(&mut self) -> io::Result<()>;
    /// The OS process ID. The pipe server checks the connecting client against it.
    fn id(&self) -> u32;
}

pub trait ProcessSpawner: Send {
    fn spawn(&mut self, exe: &Path, args: &[String]) -> io::Result<Box<dyn ChildProcess>>;
}

/// Starts processes with `std::process::Command`, with no console window.
pub struct StdSpawner;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl ProcessSpawner for StdSpawner {
    fn spawn(&mut self, exe: &Path, args: &[String]) -> io::Result<Box<dyn ChildProcess>> {
        let mut command = std::process::Command::new(exe);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let child = command.spawn()?;
        Ok(Box::new(StdChild(child)))
    }
}

struct StdChild(std::process::Child);

impl ChildProcess for StdChild {
    fn try_wait(&mut self) -> io::Result<Option<i32>> {
        Ok(self.0.try_wait()?.map(|status| status.code().unwrap_or(-1)))
    }

    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    fn id(&self) -> u32 {
        self.0.id()
    }
}
