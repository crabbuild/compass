#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub fn compat_executable() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_compass"))
}

pub fn compat_command() -> Command {
    command(compat_executable())
}

pub fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    if executable == compat_executable() {
        command
            .env("COMPASS_INTERNAL_GRAPHIFY_COMPAT", "1")
            .env("COMPASS_OUT", "graphify-out");
    }
    command
}
