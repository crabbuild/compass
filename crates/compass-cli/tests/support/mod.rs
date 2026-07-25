#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub fn compass_executable() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_compass"))
}

pub fn compass_command() -> Command {
    command(compass_executable())
}

pub fn command(executable: &Path) -> Command {
    Command::new(executable)
}
