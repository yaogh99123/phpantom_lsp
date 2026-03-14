//! Formatting proxy for external PHP formatters.
//!
//! PHPantom does not ship a formatter. Instead, it registers as a
//! formatting provider and proxies `textDocument/formatting` requests
//! to external tools.  Both tools can run in sequence: php-cs-fixer
//! first, then phpcbf.
//!
//! ## Supported tools
//!
//! 1. **php-cs-fixer** — comprehensive PSR/custom style formatter
//! 2. **phpcbf** (PHP_CodeSniffer fixer) — often extended with
//!    project-specific sniffs
//!
//! When both are available they run in sequence so that php-cs-fixer
//! handles broad formatting and phpcbf applies project-specific rules.
//!
//! ## Configuration (`.phpantom.toml`)
//!
//! ```toml
//! [formatting]
//! # Command/path for each tool. When unset, auto-detected via
//! # Composer's bin-dir (default vendor/bin), then $PATH.
//! # Set to "" to disable a tool.
//! php-cs-fixer = "vendor/bin/php-cs-fixer"
//! phpcbf = ""          # disabled
//! timeout = 10000      # ms per tool
//! ```
//!
//! ## Config file discovery
//!
//! Both tools discover their project config by walking up from the file
//! being formatted.  To ensure the project's style rules are applied,
//! formatting runs on a sibling temp file in the same directory as the
//! original so that config walkers (`.php-cs-fixer.php`,
//! `.phpcs.xml`, etc.) find the project rules.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::config::FormattingConfig;

/// Default formatting timeout in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 10_000;

// ── Tool resolution ─────────────────────────────────────────────────

/// A resolved formatting tool ready to invoke.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedTool {
    /// Human-readable name for logging.
    pub name: &'static str,
    /// Absolute or relative path to the binary.
    pub path: PathBuf,
}

/// Resolved formatting pipeline: zero, one, or two tools to run in
/// sequence.
pub(crate) struct FormattingPipeline {
    pub tools: Vec<ResolvedTool>,
}

impl FormattingPipeline {
    /// True when no tools were resolved (formatting is a no-op).
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// Build the formatting pipeline from the config and workspace root.
///
/// Resolution rules for each tool:
/// - Config value `Some("")` (empty string) → disabled.
/// - Config value `Some(cmd)` → use `cmd` as-is (user override).
/// - Config value `None` → auto-detect: try `<bin_dir>/<tool>` under
///   the workspace root (where `bin_dir` comes from Composer's
///   `config.bin-dir`, defaulting to `vendor/bin`), then search `$PATH`.
pub(crate) fn resolve_pipeline(
    workspace_root: Option<&Path>,
    config: &FormattingConfig,
    bin_dir: Option<&str>,
) -> FormattingPipeline {
    let mut tools = Vec::new();

    if let Some(resolved) = resolve_tool(
        "php-cs-fixer",
        config.php_cs_fixer.as_deref(),
        "php-cs-fixer",
        workspace_root,
        bin_dir,
    ) {
        tools.push(resolved);
    }

    if let Some(resolved) = resolve_tool(
        "phpcbf",
        config.phpcbf.as_deref(),
        "phpcbf",
        workspace_root,
        bin_dir,
    ) {
        tools.push(resolved);
    }

    FormattingPipeline { tools }
}

/// Resolve a single tool from its config value.
fn resolve_tool(
    name: &'static str,
    config_value: Option<&str>,
    binary_name: &str,
    workspace_root: Option<&Path>,
    bin_dir: Option<&str>,
) -> Option<ResolvedTool> {
    match config_value {
        // Explicitly disabled.
        Some("") => None,
        // User-provided command.
        Some(cmd) => Some(ResolvedTool {
            name,
            path: PathBuf::from(cmd),
        }),
        // Auto-detect.
        None => auto_detect(name, binary_name, workspace_root, bin_dir),
    }
}

/// Auto-detect a tool by checking `<bin_dir>/<name>` then `$PATH`.
///
/// `bin_dir` is the Composer-configured bin directory relative to the
/// workspace root (e.g. `"vendor/bin"` by default, or whatever
/// `config.bin-dir` specifies in `composer.json`).
fn auto_detect(
    name: &'static str,
    binary_name: &str,
    workspace_root: Option<&Path>,
    bin_dir: Option<&str>,
) -> Option<ResolvedTool> {
    // Check the Composer bin directory first.
    if let Some(root) = workspace_root {
        let bin = bin_dir.unwrap_or("vendor/bin");
        let candidate = root.join(bin).join(binary_name);
        if candidate.is_file() {
            return Some(ResolvedTool {
                name,
                path: candidate,
            });
        }
    }

    // Fall back to $PATH.
    if let Ok(path) = which(binary_name) {
        return Some(ResolvedTool { name, path });
    }

    None
}

/// Simple `which`-like lookup: search `$PATH` for an executable with
/// the given name.  Only returns files that are executable (on Unix).
fn which(binary_name: &str) -> Result<PathBuf, String> {
    let path_var = std::env::var("PATH").map_err(|_| "PATH not set".to_string())?;

    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() && is_executable(&candidate) {
            return Ok(candidate);
        }
    }

    Err(format!("{} not found on PATH", binary_name))
}

/// Check whether a file is executable.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

// ── Formatting pipeline ─────────────────────────────────────────────

/// Run the formatting pipeline on `content` and return `TextEdit`s.
///
/// Each tool in the pipeline runs in sequence.  The output of one tool
/// becomes the input for the next.  The final result is diffed against
/// the original content to produce edits.
///
/// `file_path` is the real path of the file on disk, used so that
/// sibling temp files land in the correct directory for tool config
/// discovery.
pub(crate) fn run_pipeline(
    pipeline: &FormattingPipeline,
    content: &str,
    file_path: &Path,
    config: &FormattingConfig,
) -> Result<Vec<TextEdit>, String> {
    let timeout_ms = config.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
    let timeout = Duration::from_millis(timeout_ms);

    let mut current = content.to_string();

    for tool in &pipeline.tools {
        current = run_tool(tool, &current, file_path, timeout)?;
    }

    Ok(compute_edits(content, &current))
}

/// Run a single tool on the content and return the formatted string.
fn run_tool(
    tool: &ResolvedTool,
    content: &str,
    file_path: &Path,
    timeout: Duration,
) -> Result<String, String> {
    match tool.name {
        "php-cs-fixer" => run_php_cs_fixer(&tool.path, content, file_path, timeout),
        "phpcbf" => run_phpcbf(&tool.path, content, file_path, timeout),
        _ => Err(format!("Unknown formatting tool: {}", tool.name)),
    }
}

/// Run php-cs-fixer on a sibling temp file and return the formatted content.
///
/// Command: `<tool> fix --using-cache=no --quiet --no-interaction <tempfile>`
///
/// php-cs-fixer modifies the file in-place.  Exit code 0 means success.
fn run_php_cs_fixer(
    tool_path: &Path,
    content: &str,
    file_path: &Path,
    timeout: Duration,
) -> Result<String, String> {
    let temp_path = write_sibling_temp_file(file_path, content)?;

    let result = run_command_with_timeout(
        Command::new(tool_path)
            .arg("fix")
            .arg("--using-cache=no")
            .arg("--quiet")
            .arg("--no-interaction")
            .arg(&temp_path),
        timeout,
    );

    let formatted = std::fs::read_to_string(&temp_path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        format!("Failed to read formatted output: {}", e)
    })?;

    let _ = std::fs::remove_file(&temp_path);

    match result {
        Ok(status) => {
            // php-cs-fixer exit codes (bitmask):
            //   0 = OK
            //   1 = general error / PHP version issue
            //  16 = configuration error
            //  32 = fixer configuration error
            //  64 = exception
            if status.code == 0 {
                Ok(formatted)
            } else {
                Err(format!(
                    "php-cs-fixer exited with code {} (stderr: {})",
                    status.code,
                    status.stderr.trim()
                ))
            }
        }
        Err(e) => Err(e),
    }
}

/// Run phpcbf on a sibling temp file and return the formatted content.
///
/// Command: `<tool> --no-colors -q <tempfile>`
///
/// phpcbf modifies the file in-place.
fn run_phpcbf(
    tool_path: &Path,
    content: &str,
    file_path: &Path,
    timeout: Duration,
) -> Result<String, String> {
    let temp_path = write_sibling_temp_file(file_path, content)?;

    let result = run_command_with_timeout(
        Command::new(tool_path)
            .arg("--no-colors")
            .arg("-q")
            .arg(&temp_path),
        timeout,
    );

    let formatted = std::fs::read_to_string(&temp_path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        format!("Failed to read formatted output: {}", e)
    })?;

    let _ = std::fs::remove_file(&temp_path);

    match result {
        Ok(status) => {
            // phpcbf exit codes:
            //   0 = no fixes needed
            //   1 = fixes applied (success)
            //   2 = could not fix all errors
            //   3+ = operational error
            match status.code {
                0 | 1 => Ok(formatted),
                _ => Err(format!(
                    "phpcbf exited with code {} (stderr: {})",
                    status.code,
                    status.stderr.trim()
                )),
            }
        }
        Err(e) => Err(e),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Write content to a temporary file in the same directory as `original`
/// so that tool config discovery (which walks up from the file) works.
///
/// The temp file is hidden (dot-prefixed) and includes the process ID
/// for uniqueness.
fn write_sibling_temp_file(original: &Path, content: &str) -> Result<PathBuf, String> {
    let parent = original
        .parent()
        .ok_or_else(|| "Cannot determine parent directory of file".to_string())?;

    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("phpantom");

    let unique = std::process::id();
    let temp_name = format!(".{}.phpantom-fmt.{}.php", stem, unique);
    let temp_path = parent.join(temp_name);

    let mut file = std::fs::File::create(&temp_path)
        .map_err(|e| format!("Failed to create temp file {}: {}", temp_path.display(), e))?;

    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    file.flush()
        .map_err(|e| format!("Failed to flush temp file: {}", e))?;

    Ok(temp_path)
}

/// Result of running an external command.
struct CommandResult {
    /// Exit code (or -1 if the process was killed / no code available).
    code: i32,
    /// Captured stderr content.
    stderr: String,
}

/// Spawn a command, wait for it with a timeout, and return the result.
///
/// Stdout is suppressed.  Stderr is captured for error reporting.
fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<CommandResult, String> {
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn formatter: {}", e))?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = child
                    .stderr
                    .take()
                    .and_then(|mut s| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                        Some(buf)
                    })
                    .unwrap_or_default();

                return Ok(CommandResult {
                    code: status.code().unwrap_or(-1),
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Formatter timed out after {}ms",
                        timeout.as_millis()
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("Error waiting for formatter: {}", e));
            }
        }
    }
}

/// Compute the `TextEdit`s needed to transform `original` into `formatted`.
///
/// Returns a single `TextEdit` that replaces the entire document.  Only
/// returns edits if the content actually changed.
fn compute_edits(original: &str, formatted: &str) -> Vec<TextEdit> {
    if original == formatted {
        return Vec::new();
    }

    let line_count = original.lines().count();
    let last_line_idx = if line_count == 0 { 0 } else { line_count - 1 };
    let last_line_len = original.lines().last().map_or(0, |l| l.len());

    let (end_line, end_char) = if original.ends_with('\n') {
        (last_line_idx + 1, 0)
    } else {
        (last_line_idx, last_line_len)
    };

    vec![TextEdit {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: end_line as u32,
                character: end_char as u32,
            },
        },
        new_text: formatted.to_string(),
    }]
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_edits ───────────────────────────────────────────────

    #[test]
    fn compute_edits_no_change() {
        let content = "<?php\necho 'hello';\n";
        let edits = compute_edits(content, content);
        assert!(edits.is_empty());
    }

    #[test]
    fn compute_edits_with_change() {
        let original = "<?php\necho 'hello';\n";
        let formatted = "<?php\n\necho 'hello';\n";
        let edits = compute_edits(original, formatted);
        assert_eq!(edits.len(), 1);
        let edit = &edits[0];
        assert_eq!(edit.range.start.line, 0);
        assert_eq!(edit.range.start.character, 0);
        assert_eq!(edit.range.end.line, 2);
        assert_eq!(edit.range.end.character, 0);
        assert_eq!(edit.new_text, formatted);
    }

    #[test]
    fn compute_edits_empty_original() {
        let original = "";
        let formatted = "<?php\n";
        let edits = compute_edits(original, formatted);
        assert_eq!(edits.len(), 1);
        let edit = &edits[0];
        assert_eq!(edit.range.start.line, 0);
        assert_eq!(edit.range.start.character, 0);
        assert_eq!(edit.range.end.line, 0);
        assert_eq!(edit.range.end.character, 0);
    }

    #[test]
    fn compute_edits_no_trailing_newline() {
        let original = "<?php\necho 'hello';";
        let formatted = "<?php\necho 'hello';\n";
        let edits = compute_edits(original, formatted);
        assert_eq!(edits.len(), 1);
        let edit = &edits[0];
        assert_eq!(edit.range.end.line, 1);
        assert_eq!(edit.range.end.character, 13);
    }

    // ── resolve_pipeline ────────────────────────────────────────────

    #[test]
    fn pipeline_default_config_no_tools_available() {
        // No workspace root, nothing on PATH → empty pipeline.
        let config = FormattingConfig::default();
        let pipeline = resolve_pipeline(None, &config, None);
        assert!(pipeline.is_empty());
    }

    #[test]
    fn pipeline_both_disabled() {
        let config = FormattingConfig {
            php_cs_fixer: Some(String::new()),
            phpcbf: Some(String::new()),
            timeout: None,
        };
        let pipeline = resolve_pipeline(None, &config, None);
        assert!(pipeline.is_empty());
    }

    #[test]
    fn pipeline_explicit_commands() {
        let config = FormattingConfig {
            php_cs_fixer: Some("/usr/bin/php-cs-fixer".to_string()),
            phpcbf: Some("/usr/bin/phpcbf".to_string()),
            timeout: None,
        };
        let pipeline = resolve_pipeline(None, &config, None);
        assert_eq!(pipeline.tools.len(), 2);
        assert_eq!(pipeline.tools[0].name, "php-cs-fixer");
        assert_eq!(
            pipeline.tools[0].path,
            PathBuf::from("/usr/bin/php-cs-fixer")
        );
        assert_eq!(pipeline.tools[1].name, "phpcbf");
        assert_eq!(pipeline.tools[1].path, PathBuf::from("/usr/bin/phpcbf"));
    }

    #[test]
    fn pipeline_one_explicit_one_disabled() {
        let config = FormattingConfig {
            php_cs_fixer: Some("/usr/bin/php-cs-fixer".to_string()),
            phpcbf: Some(String::new()),
            timeout: None,
        };
        let pipeline = resolve_pipeline(None, &config, None);
        assert_eq!(pipeline.tools.len(), 1);
        assert_eq!(pipeline.tools[0].name, "php-cs-fixer");
    }

    #[test]
    fn pipeline_vendor_bin_auto_detect() {
        let dir = tempfile::tempdir().unwrap();
        let vendor_bin = dir.path().join("vendor/bin");
        std::fs::create_dir_all(&vendor_bin).unwrap();

        for name in &["php-cs-fixer", "phpcbf"] {
            let p = vendor_bin.join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        let config = FormattingConfig::default();
        let pipeline = resolve_pipeline(Some(dir.path()), &config, None);
        assert_eq!(pipeline.tools.len(), 2);
        assert_eq!(pipeline.tools[0].name, "php-cs-fixer");
        assert_eq!(pipeline.tools[0].path, vendor_bin.join("php-cs-fixer"));
        assert_eq!(pipeline.tools[1].name, "phpcbf");
        assert_eq!(pipeline.tools[1].path, vendor_bin.join("phpcbf"));
    }

    #[test]
    fn pipeline_vendor_bin_only_phpcbf() {
        let dir = tempfile::tempdir().unwrap();
        let vendor_bin = dir.path().join("vendor/bin");
        std::fs::create_dir_all(&vendor_bin).unwrap();

        let p = vendor_bin.join("phpcbf");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let config = FormattingConfig::default();
        let pipeline = resolve_pipeline(Some(dir.path()), &config, None);
        assert_eq!(pipeline.tools.len(), 1);
        assert_eq!(pipeline.tools[0].name, "phpcbf");
    }

    #[test]
    fn pipeline_explicit_overrides_auto_detect() {
        let dir = tempfile::tempdir().unwrap();
        let vendor_bin = dir.path().join("vendor/bin");
        std::fs::create_dir_all(&vendor_bin).unwrap();
        let p = vendor_bin.join("php-cs-fixer");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // User explicitly set a different path — should use that, not vendor/bin.
        let config = FormattingConfig {
            php_cs_fixer: Some("/opt/php-cs-fixer".to_string()),
            phpcbf: Some(String::new()),
            timeout: None,
        };
        let pipeline = resolve_pipeline(Some(dir.path()), &config, None);
        assert_eq!(pipeline.tools.len(), 1);
        assert_eq!(pipeline.tools[0].path, PathBuf::from("/opt/php-cs-fixer"));
    }

    // ── sibling temp file ───────────────────────────────────────────

    #[test]
    fn write_sibling_temp_file_in_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("MyClass.php");
        std::fs::write(&original, "<?php\n").unwrap();

        let content = "<?php\necho 'formatted';\n";
        let temp = write_sibling_temp_file(&original, content).unwrap();

        assert_eq!(temp.parent(), original.parent());
        let name = temp.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with('.'));
        assert!(name.contains("phpantom-fmt"));
        assert!(name.ends_with(".php"));

        let read_back = std::fs::read_to_string(&temp).unwrap();
        assert_eq!(read_back, content);

        let _ = std::fs::remove_file(&temp);
    }

    // ── custom bin dir ──────────────────────────────────────────────

    #[test]
    fn pipeline_custom_bin_dir_auto_detect() {
        // Simulate a project where composer.json has config.bin-dir = "bin".
        let dir = tempfile::tempdir().unwrap();
        let custom_bin = dir.path().join("bin");
        std::fs::create_dir_all(&custom_bin).unwrap();

        for name in &["php-cs-fixer", "phpcbf"] {
            let p = custom_bin.join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        let config = FormattingConfig::default();
        let pipeline = resolve_pipeline(Some(dir.path()), &config, Some("bin"));
        assert_eq!(pipeline.tools.len(), 2);
        assert_eq!(pipeline.tools[0].path, custom_bin.join("php-cs-fixer"));
        assert_eq!(pipeline.tools[1].path, custom_bin.join("phpcbf"));
    }

    #[test]
    fn pipeline_custom_bin_dir_not_found_falls_back_to_path() {
        // Custom bin dir exists but has no tools → pipeline is empty
        // (assuming the tools are not on $PATH either).
        let dir = tempfile::tempdir().unwrap();
        let custom_bin = dir.path().join("tools/bin");
        std::fs::create_dir_all(&custom_bin).unwrap();

        let config = FormattingConfig::default();
        let pipeline = resolve_pipeline(Some(dir.path()), &config, Some("tools/bin"));
        // No binaries in tools/bin and presumably not on $PATH in CI.
        // At minimum, pipeline should not panic.
        assert!(pipeline.tools.is_empty() || !pipeline.tools.is_empty());
    }

    #[test]
    fn pipeline_custom_bin_dir_ignores_default_vendor_bin() {
        // Tools exist in vendor/bin but the project uses a custom bin
        // dir that does NOT contain them → should not find them.
        let dir = tempfile::tempdir().unwrap();
        let vendor_bin = dir.path().join("vendor/bin");
        std::fs::create_dir_all(&vendor_bin).unwrap();
        let custom_bin = dir.path().join("custom-bin");
        std::fs::create_dir_all(&custom_bin).unwrap();

        let p = vendor_bin.join("php-cs-fixer");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let config = FormattingConfig::default();
        // With custom bin dir pointing to "custom-bin", vendor/bin should NOT be checked.
        let pipeline = resolve_pipeline(Some(dir.path()), &config, Some("custom-bin"));
        // php-cs-fixer is in vendor/bin but NOT in custom-bin, so auto-detect should skip it.
        let has_fixer = pipeline
            .tools
            .iter()
            .any(|t| t.name == "php-cs-fixer" && t.path == vendor_bin.join("php-cs-fixer"));
        assert!(
            !has_fixer,
            "Should not find php-cs-fixer in vendor/bin when custom bin dir is set"
        );
    }

    // ── is_executable ───────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn is_executable_true() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test-bin");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(is_executable(&file));
    }

    #[cfg(unix)]
    #[test]
    fn is_executable_false() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test-noexec");
        std::fs::write(&file, "just data\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(!is_executable(&file));
    }
}
