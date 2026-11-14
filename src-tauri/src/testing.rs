//! What the unit tests share and nothing else uses.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Writes an executable stand-in without this process ever holding it open
/// for writing.
///
/// Every unit test runs in one process, and plenty of them start programs. A
/// fork copies every descriptor the process has open at that moment, and the
/// child keeps its copies until it execs. Linux refuses to run a file that
/// anything holds open for writing, so a script written here and run straight
/// away failed with "Text file busy" whenever another test forked while it
/// was still open, and the program under test read that as a program that
/// would not start. `sh` writes it instead, and has exited before anybody
/// runs it.
pub(crate) fn write_executable(path: &Path, contents: &str) {
    let mut writer = Command::new("/bin/sh")
        .arg("-c")
        .arg("cat > \"$0\" && chmod 755 \"$0\"")
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    writer.stdin.take().unwrap().write_all(contents.as_bytes()).unwrap();
    assert!(writer.wait().unwrap().success(), "could not write {}", path.display());
}
