//! Input validation for osmoda-watch.
//!
//! Shared validators for health checks and watcher actions.
//! Applied at both registration time (API handlers) and execution time.

use crate::switch::HealthCheck;
use crate::watcher::WatchAction;

/// Validate a systemd unit name — only safe characters allowed.
pub fn validate_unit_name(unit: &str) -> Result<(), String> {
    if unit.is_empty() || unit.len() > 256 {
        return Err("unit name must be 1-256 characters".to_string());
    }
    if !unit
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '@' || c == '.' || c == '_' || c == '-')
    {
        return Err(format!("invalid characters in unit name: {unit}"));
    }
    Ok(())
}

/// Validate that a command path is allowed. Rejects shell interpreters and
/// requires absolute paths to prevent ambient PATH abuse.
pub fn validate_command(cmd: &str) -> Result<(), String> {
    if !cmd.starts_with('/') {
        return Err(format!("command must be an absolute path, got: {cmd}"));
    }

    let basename = cmd.rsplit('/').next().unwrap_or(cmd);
    let blocked_basenames = [
        "sh", "bash", "zsh", "dash", "fish", "csh", "tcsh", "env", "python", "python3", "perl",
        "ruby", "node", "lua",
    ];

    if blocked_basenames.contains(&basename) {
        return Err(format!(
            "shell interpreters are blocked for security: {cmd}"
        ));
    }

    if cmd.contains("..") {
        return Err("command path must not contain '..'".to_string());
    }

    Ok(())
}

/// Validate that a URL uses an allowed scheme (http/https only).
pub fn validate_url(url: &str) -> Result<(), String> {
    let lower = url.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Ok(())
    } else {
        Err(format!(
            "URL must use http:// or https:// scheme, got: {}",
            url.split("://").next().unwrap_or("unknown")
        ))
    }
}

/// Validate that command arguments don't contain shell metacharacters.
/// These characters could be exploited if args are ever passed through a shell.
pub fn validate_args(args: &[String]) -> Result<(), String> {
    const SHELL_METACHARS: &[char] = &['$', '`', '|', ';', '&', '(', ')', '{', '}', '<', '>', '!', '\\', '\n', '\r', '\0'];
    for arg in args {
        if let Some(bad) = arg.chars().find(|c| SHELL_METACHARS.contains(c)) {
            return Err(format!("argument contains shell metacharacter '{bad}': {arg}"));
        }
    }
    Ok(())
}

/// Validate a single health check definition.
pub fn validate_health_check(check: &HealthCheck) -> Result<(), String> {
    match check {
        HealthCheck::SystemdUnit { unit } => validate_unit_name(unit),
        HealthCheck::Command { cmd, args } => {
            validate_command(cmd)?;
            validate_args(args)
        }
        HealthCheck::HttpGet { url, .. } => validate_url(url),
        HealthCheck::TcpPort { .. } => Ok(()),
    }
}

/// Validate a watcher action.
pub fn validate_watch_action(action: &WatchAction) -> Result<(), String> {
    match action {
        WatchAction::RestartService { unit } => validate_unit_name(unit),
        WatchAction::RollbackGeneration => Ok(()),
        WatchAction::Notify { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_unit_name_valid() {
        assert!(validate_unit_name("sshd").is_ok());
        assert!(validate_unit_name("osmoda-agentd.service").is_ok());
        assert!(validate_unit_name("foo@bar.service").is_ok());
        assert!(validate_unit_name("a_b-c.d").is_ok());
    }

    #[test]
    fn test_validate_unit_name_rejects_injection() {
        assert!(validate_unit_name("").is_err());
        assert!(validate_unit_name("foo; rm -rf /").is_err());
        assert!(validate_unit_name("foo$(whoami)").is_err());
        assert!(validate_unit_name("foo`id`").is_err());
        assert!(validate_unit_name("../etc/passwd").is_err());
    }

    #[test]
    fn test_validate_command_requires_absolute_path() {
        assert!(validate_command("relative/path").is_err());
        assert!(validate_command("just-a-name").is_err());
        assert!(validate_command("/usr/bin/systemctl").is_ok());
    }

    #[test]
    fn test_validate_command_blocks_interpreters() {
        assert!(validate_command("/bin/sh").is_err());
        assert!(validate_command("/usr/bin/bash").is_err());
        assert!(validate_command("/usr/bin/python3").is_err());
        assert!(validate_command("/usr/bin/env").is_err());
        assert!(validate_command("/nix/store/abc123-bash/bin/bash").is_err());
    }

    #[test]
    fn test_validate_command_blocks_path_traversal() {
        assert!(validate_command("/usr/bin/../bin/sh").is_err());
    }

    #[test]
    fn test_validate_command_allows_safe_commands() {
        assert!(validate_command("/usr/bin/systemctl").is_ok());
        assert!(validate_command("/run/current-system/sw/bin/nixos-rebuild").is_ok());
        assert!(validate_command("/usr/bin/curl").is_ok());
    }

    #[test]
    fn test_validate_url_valid() {
        assert!(validate_url("http://localhost:8080/health").is_ok());
        assert!(validate_url("https://example.com").is_ok());
    }

    #[test]
    fn test_validate_url_rejects_dangerous_schemes() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("gopher://evil.com").is_err());
        assert!(validate_url("ftp://server/file").is_err());
    }

    #[test]
    fn test_validate_health_check() {
        assert!(validate_health_check(&HealthCheck::SystemdUnit {
            unit: "sshd".to_string()
        })
        .is_ok());
        assert!(validate_health_check(&HealthCheck::SystemdUnit {
            unit: "foo; rm -rf /".to_string()
        })
        .is_err());
        assert!(validate_health_check(&HealthCheck::Command {
            cmd: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "whoami".to_string()],
        })
        .is_err());
        assert!(validate_health_check(&HealthCheck::HttpGet {
            url: "file:///etc/shadow".to_string(),
            expect_status: 200,
        })
        .is_err());
    }

    #[test]
    fn test_validate_args_safe() {
        assert!(validate_args(&["--flag".to_string(), "value".to_string()]).is_ok());
        assert!(validate_args(&["/path/to/file".to_string()]).is_ok());
        assert!(validate_args(&["simple-arg".to_string()]).is_ok());
    }

    #[test]
    fn test_validate_args_rejects_metacharacters() {
        assert!(validate_args(&["$(whoami)".to_string()]).is_err());
        assert!(validate_args(&["`id`".to_string()]).is_err());
        assert!(validate_args(&["foo|bar".to_string()]).is_err());
        assert!(validate_args(&["foo;rm -rf /".to_string()]).is_err());
        assert!(validate_args(&["foo&bg".to_string()]).is_err());
        assert!(validate_args(&["a\nb".to_string()]).is_err());
    }

    #[test]
    fn test_validate_health_check_rejects_bad_args() {
        assert!(validate_health_check(&HealthCheck::Command {
            cmd: "/usr/bin/systemctl".to_string(),
            args: vec!["status".to_string(), "$(whoami)".to_string()],
        })
        .is_err());
    }

    #[test]
    fn test_validate_watch_action() {
        assert!(validate_watch_action(&WatchAction::RestartService {
            unit: "sshd".to_string()
        })
        .is_ok());
        assert!(validate_watch_action(&WatchAction::RestartService {
            unit: "foo$(whoami)".to_string()
        })
        .is_err());
        assert!(validate_watch_action(&WatchAction::RollbackGeneration).is_ok());
    }
}
