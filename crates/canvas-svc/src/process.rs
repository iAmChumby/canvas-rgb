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

/// Windows exit codes that name a known NTSTATUS, by value.
const EXIT_NAMES: &[(u32, &str)] = &[
    (0x4001_0004, "DBG_TERMINATE_PROCESS"),
    (0x4001_0005, "DBG_CONTROL_C"),
    (0x8000_0003, "STATUS_BREAKPOINT"),
    (0xC000_0005, "STATUS_ACCESS_VIOLATION"),
    (0xC000_0006, "STATUS_IN_PAGE_ERROR"),
    (0xC000_001D, "STATUS_ILLEGAL_INSTRUCTION"),
    (0xC000_0022, "STATUS_ACCESS_DENIED"),
    (0xC000_0094, "STATUS_INTEGER_DIVIDE_BY_ZERO"),
    (0xC000_0096, "STATUS_PRIVILEGED_INSTRUCTION"),
    (0xC000_00FD, "STATUS_STACK_OVERFLOW"),
    (0xC000_0135, "STATUS_DLL_NOT_FOUND"),
    (0xC000_0139, "STATUS_ENTRYPOINT_NOT_FOUND"),
    (0xC000_013A, "STATUS_CONTROL_C_EXIT"),
    (0xC000_0142, "STATUS_DLL_INIT_FAILED"),
    (0xC000_0194, "STATUS_POSSIBLE_DEADLOCK"),
    (0xC000_0374, "STATUS_HEAP_CORRUPTION"),
    (0xC000_0409, "STATUS_STACK_BUFFER_OVERRUN"),
    (0xC000_0420, "STATUS_ASSERTION_FAILURE"),
];

/// A child's exit code as the logs show it: hex, then the NTSTATUS name when one is known
/// (`0xC0000142 STATUS_DLL_INIT_FAILED`). `GetExitCodeProcess` returns a `u32`, but
/// [`ChildProcess::try_wait`] carries it as an `i32`, which turned `0xC0000142` into
/// `-1073741502` in the log; the cast back restores the bits.
pub fn describe_exit_code(code: i32) -> String {
    let bits = code as u32;
    match EXIT_NAMES.iter().find(|(value, _)| *value == bits) {
        Some((_, name)) => format!("0x{bits:08X} {name}"),
        None => format!("0x{bits:08X}"),
    }
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

#[cfg(test)]
mod tests {
    use super::describe_exit_code;

    #[test]
    fn exit_codes_are_hex_with_the_ntstatus_name_where_known() {
        // The two codes from #19, as the service saw them (decimal in the old log).
        assert_eq!(describe_exit_code(1_073_807_364), "0x40010004 DBG_TERMINATE_PROCESS");
        assert_eq!(describe_exit_code(-1_073_741_502), "0xC0000142 STATUS_DLL_INIT_FAILED");
        // Every table entry round-trips through the i32 the process seam carries.
        for (value, name) in super::EXIT_NAMES {
            assert_eq!(describe_exit_code(*value as i32), format!("0x{value:08X} {name}"));
        }
        assert_eq!(describe_exit_code(0xC000_0005_u32 as i32), "0xC0000005 STATUS_ACCESS_VIOLATION");
        assert_eq!(describe_exit_code(0xC000_0409_u32 as i32), "0xC0000409 STATUS_STACK_BUFFER_OVERRUN");
        // Plain codes get hex and no invented name.
        assert_eq!(describe_exit_code(1), "0x00000001");
        assert_eq!(describe_exit_code(0), "0x00000000");
        assert_eq!(describe_exit_code(-1), "0xFFFFFFFF");
    }

    #[test]
    fn the_name_table_has_no_duplicate_codes() {
        let mut values: Vec<u32> = super::EXIT_NAMES.iter().map(|(v, _)| *v).collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), super::EXIT_NAMES.len());
    }
}
