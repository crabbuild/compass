use std::str::FromStr;

use compass_ocr::{ModelProfile, install_profile, list_profiles, verify_profile};

use crate::Outcome;

pub(crate) fn command(args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        Some("list") => list(&args[1..]),
        Some("install") => profile_command(&args[1..], true),
        Some("verify") => profile_command(&args[1..], false),
        Some(other) => usage_error(&format!("unknown models command {other:?}")),
        None => usage_error("missing models command"),
    }
}

fn list(args: &[String]) -> Outcome {
    let format = match args {
        [] => "text",
        [flag, value] if flag == "--format" && matches!(value.as_str(), "text" | "json") => value,
        _ => return usage_error("models list accepts only --format text|json"),
    };
    let statuses = match list_profiles() {
        Ok(statuses) => statuses,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if format == "json" {
        return match serde_json::to_string_pretty(&serde_json::json!({
            "schema": "compass.models/1",
            "profiles": statuses,
        })) {
            Ok(json) => Outcome::success(json),
            Err(error) => {
                Outcome::failure(format!("error: could not encode model status: {error}"))
            }
        };
    }
    let output = statuses
        .into_iter()
        .map(|status| {
            format!(
                "{}\t{}\t{} MiB\t{}",
                status.profile,
                if status.verified {
                    "installed and verified"
                } else if status.installed {
                    "installed but invalid"
                } else {
                    "not installed"
                },
                status.bytes / (1024 * 1024),
                status.license
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::success(output)
}

fn profile_command(args: &[String], install: bool) -> Outcome {
    let [name] = args else {
        return usage_error(if install {
            "models install requires one pinned profile"
        } else {
            "models verify requires one pinned profile"
        });
    };
    let profile = match ModelProfile::from_str(name) {
        Ok(profile) => profile,
        Err(error) => return usage_error(&error.to_string()),
    };
    let result = if install {
        install_profile(profile)
    } else {
        verify_profile(profile)
    };
    match result {
        Ok(files) => Outcome::success(format!(
            "{} is installed and verified for {} {} ({} model artifacts)",
            profile.name(),
            files.identity.engine,
            files.identity.engine_version,
            files.identity.model_digests.len()
        )),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn usage_error(message: &str) -> Outcome {
    Outcome::from_command_output(
        2,
        String::new(),
        format!(
            "error: {message}\nusage: compass models <list|install|verify> [PROFILE] [--format text|json]"
        ),
    )
}
