use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
