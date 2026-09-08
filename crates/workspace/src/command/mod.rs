use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use async_process::Command;
use futures_util::io::{AsyncRead, AsyncReadExt as _};
use ignore::WalkBuilder;
use magenta_core::{
    WorkspaceCommand, WorkspaceCommandError, WorkspaceCommandEvent, WorkspaceCommandOutputStream,
    WorkspaceCommandResult, WorkspaceCommandRunner, WorkspaceCommandStatus, WorkspaceCommandStream,
};

use crate::path;

const DEFAULT_TIMEOUT_SECONDS: u16 = 120;
const MAX_TIMEOUT_SECONDS: u16 = 600;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_CAPTURE_BYTES: usize = 32 * 1024;
const MAX_UI_BYTES: usize = 128 * 1024;
const MAX_MASKS: usize = 512;

#[derive(Clone, Debug)]
pub struct BubblewrapCommandRunner {
    executable: PathBuf,
    cargo_home: Option<PathBuf>,
    rustup_home: Option<PathBuf>,
    node_home: Option<PathBuf>,
    pnpm_home: Option<PathBuf>,
    pnpm_cache: Option<PathBuf>,
    pnpm_with_current: bool,
}

impl BubblewrapCommandRunner {
    /// Locates Bubblewrap and verifies that a minimal isolated process can start.
    ///
    /// # Errors
    /// Returns an error when Bubblewrap is missing or user namespaces are unavailable.
    pub fn new() -> Result<Self, WorkspaceCommandError> {
        let executable = find_executable("bwrap").ok_or_else(|| {
            WorkspaceCommandError::new(io::Error::new(
                io::ErrorKind::NotFound,
                "bubblewrap is unavailable",
            ))
        })?;
        probe(&executable).map_err(WorkspaceCommandError::new)?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".cargo")))
            .filter(|path| path.is_dir());
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".rustup")))
            .filter(|path| path.is_dir());
        let node_home = executable_root("node");
        let pnpm_home = pnpm_root();
        let pnpm_cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".cache")))
            .map(|cache| cache.join("pnpm/v11"))
            .filter(|path| path.is_dir());
        let pnpm_with_current = executable_major_version("pnpm").is_some_and(|major| major >= 11);
        Ok(Self {
            executable,
            cargo_home,
            rustup_home,
            node_home,
            pnpm_home,
            pnpm_cache,
            pnpm_with_current,
        })
    }

    fn execute(&self, root: PathBuf, command: WorkspaceCommand) -> WorkspaceCommandStream {
        let runner = self.clone();
        let (sender, receiver) = async_channel::unbounded();
        smol::spawn(async move {
            if let Err(error) = runner.execute_inner(&root, &command, &sender).await {
                let _ = sender.send(Err(WorkspaceCommandError::new(error))).await;
            }
        })
        .detach();

        Box::pin(async_stream::stream! {
            while let Ok(event) = receiver.recv().await {
                yield event;
            }
        })
    }

    async fn execute_inner(
        &self,
        root: &Path,
        command: &WorkspaceCommand,
        sender: &async_channel::Sender<Result<WorkspaceCommandEvent, WorkspaceCommandError>>,
    ) -> io::Result<()> {
        let prepared = PreparedCommand::new(root, command)?;
        let masks = protected_masks(&prepared.root)?;
        let pnpm_scratch = if prepared.program == "pnpm" {
            self.prepare_pnpm_scratch()?
        } else {
            None
        };
        let mut process = Command::new(&self.executable);
        configure_sandbox(&mut process, &prepared, &masks, self, pnpm_scratch.as_ref());
        process
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let started = Instant::now();
        let mut child = process.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("command stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("command stderr was not captured"))?;
        let stdout_task = smol::spawn(read_output(
            stdout,
            WorkspaceCommandOutputStream::Stdout,
            sender.clone(),
        ));
        let stderr_task = smol::spawn(read_output(
            stderr,
            WorkspaceCommandOutputStream::Stderr,
            sender.clone(),
        ));
        let timeout = Duration::from_secs(u64::from(prepared.timeout_seconds));
        let (status, terminal) = loop {
            if sender.is_closed() {
                let _ = child.kill();
                let status = child.status().await?;
                break (status, WorkspaceCommandStatus::Cancelled);
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let status = child.status().await?;
                break (status, WorkspaceCommandStatus::TimedOut);
            }
            if let Some(status) = child.try_status()? {
                break (status, WorkspaceCommandStatus::Exited);
            }
            smol::Timer::after(Duration::from_millis(25)).await;
        };
        let stdout = stdout_task.await?;
        let stderr = stderr_task.await?;
        if terminal == WorkspaceCommandStatus::Cancelled || sender.is_closed() {
            return Ok(());
        }
        let truncated = stdout.truncated || stderr.truncated;
        let stdout = stdout.text();
        let stderr = stderr.text();
        let terminal = if terminal == WorkspaceCommandStatus::Exited
            && !status.success()
            && stderr.trim_start().starts_with("bwrap:")
        {
            WorkspaceCommandStatus::Failed
        } else {
            terminal
        };
        sender
            .send(Ok(WorkspaceCommandEvent::Finished(
                WorkspaceCommandResult {
                    status: terminal,
                    exit_code: status.code(),
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    truncated,
                    stdout,
                    stderr,
                },
            )))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "command receiver closed"))
    }

    fn prepare_pnpm_scratch(&self) -> io::Result<Option<PnpmScratch>> {
        let Some(home) = &self.pnpm_home else {
            return Ok(None);
        };
        let source = home.join("store/v11");
        let source_index = source.join("index.db");
        let source_files = source.join("files");
        if !source_index.is_file() || !source_files.is_dir() {
            return Ok(None);
        }
        let directory = tempfile::tempdir()?;
        fs::copy(&source_index, directory.path().join("index.db"))?;
        fs::create_dir(directory.path().join("files"))?;
        Ok(Some(PnpmScratch {
            directory,
            source_files,
            source_cache: self.pnpm_cache.clone(),
        }))
    }
}

impl WorkspaceCommandRunner for BubblewrapCommandRunner {
    fn run(&self, root: PathBuf, command: WorkspaceCommand) -> WorkspaceCommandStream {
        self.execute(root, command)
    }
}

struct PreparedCommand {
    root: PathBuf,
    cwd: String,
    program: String,
    args: Vec<String>,
    timeout_seconds: u16,
}

impl PreparedCommand {
    fn new(root: &Path, command: &WorkspaceCommand) -> io::Result<Self> {
        let root = path::canonical_root(root)?;
        validate_program(&command.program)?;
        if command.args.len() > MAX_ARGUMENTS
            || command.args.iter().map(String::len).sum::<usize>() > MAX_ARGUMENT_BYTES
            || command.args.iter().any(|arg| arg.contains('\0'))
        {
            return Err(invalid("command arguments exceed the supported limits"));
        }
        let cwd = if command.cwd.trim().is_empty() {
            "."
        } else {
            command.cwd.as_str()
        };
        let directory = if cwd == "." {
            root.clone()
        } else {
            path::safe_path(&root, cwd, true)?
        };
        if !directory.is_dir() {
            return Err(invalid("command working directory must be a directory"));
        }
        let timeout_seconds = if command.timeout_seconds == 0 {
            DEFAULT_TIMEOUT_SECONDS
        } else if command.timeout_seconds <= MAX_TIMEOUT_SECONDS {
            command.timeout_seconds
        } else {
            return Err(invalid("command timeout exceeds 600 seconds"));
        };
        Ok(Self {
            root,
            cwd: cwd.to_owned(),
            program: command.program.clone(),
            args: command.args.clone(),
            timeout_seconds,
        })
    }
}

fn validate_program(program: &str) -> io::Result<()> {
    let path = Path::new(program);
    if program.trim().is_empty() || program.contains('\0') || path.is_absolute() {
        return Err(invalid("command executable must be a relative name"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(invalid("command executable cannot escape the workspace"));
    }
    Ok(())
}

fn configure_sandbox(
    process: &mut Command,
    command: &PreparedCommand,
    masks: &[Mask],
    runner: &BubblewrapCommandRunner,
    pnpm_scratch: Option<&PnpmScratch>,
) {
    configure_isolation(process);
    process.arg("--bind").arg(&command.root).arg("/workspace");
    configure_masks(process, masks);
    let path_value = configure_toolchain(process, runner, pnpm_scratch);
    configure_invocation(process, command, path_value, runner.pnpm_with_current);
}

fn configure_isolation(process: &mut Command) {
    process.args([
        "--unshare-all",
        "--unshare-user",
        "--die-with-parent",
        "--new-session",
        "--disable-userns",
        "--cap-drop",
        "ALL",
        "--clearenv",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/tmp/home",
        "--dir",
        "/workspace",
        "--ro-bind-try",
        "/usr",
        "/usr",
        "--ro-bind-try",
        "/bin",
        "/bin",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--ro-bind-try",
        "/etc",
        "/etc",
    ]);
}

fn configure_masks(process: &mut Command, masks: &[Mask]) {
    for mask in masks {
        match mask.kind {
            MaskKind::Directory => {
                process.arg("--tmpfs").arg(&mask.destination);
                process.arg("--remount-ro").arg(&mask.destination);
            }
            MaskKind::File => {
                process
                    .arg("--ro-bind")
                    .arg("/dev/null")
                    .arg(&mask.destination);
            }
        }
    }
}

fn configure_toolchain(
    process: &mut Command,
    runner: &BubblewrapCommandRunner,
    pnpm_scratch: Option<&PnpmScratch>,
) -> String {
    process.args([
        "--dir",
        "/toolchain",
        "--dir",
        "/toolcache",
        "--dir",
        "/toolcache/cargo",
    ]);
    let mut path_entries = Vec::new();
    if let Some(node_home) = &runner.node_home {
        process
            .arg("--ro-bind")
            .arg(node_home)
            .arg("/toolchain/node");
        path_entries.push("/toolchain/node/bin");
    }
    if let Some(pnpm_home) = &runner.pnpm_home {
        process
            .arg("--ro-bind")
            .arg(pnpm_home)
            .arg("/toolchain/pnpm");
        process.args(["--setenv", "PNPM_HOME", "/toolchain/pnpm/bin"]);
        if let Some(scratch) = pnpm_scratch {
            process
                .arg("--bind")
                .arg(scratch.directory.path())
                .arg("/toolcache/pnpm-store")
                .arg("--ro-bind")
                .arg(&scratch.source_files)
                .arg("/toolcache/pnpm-store/files");
            if let Some(source_cache) = &scratch.source_cache {
                process.args([
                    "--dir",
                    "/tmp/home/.cache",
                    "--dir",
                    "/tmp/home/.cache/pnpm",
                ]);
                process
                    .arg("--ro-bind")
                    .arg(source_cache)
                    .arg("/tmp/home/.cache/pnpm/v11");
            }
            process.args(["--setenv", "pnpm_config_store_dir", "/toolcache/pnpm-store"]);
        }
        process.args([
            "--setenv",
            "pnpm_config_pm_on_fail",
            "ignore",
            "--setenv",
            "pnpm_config_offline",
            "true",
            "--setenv",
            "COREPACK_HOME",
            "/tmp/corepack",
        ]);
        path_entries.push("/toolchain/pnpm/bin");
    }
    if let Some(cargo_home) = &runner.cargo_home {
        let binaries = cargo_home.join("bin");
        if binaries.is_dir() {
            process
                .arg("--ro-bind")
                .arg(binaries)
                .arg("/toolchain/cargo-bin");
            path_entries.push("/toolchain/cargo-bin");
        }
        for directory in ["registry", "git"] {
            let source = cargo_home.join(directory);
            if source.is_dir() {
                process
                    .arg("--ro-bind")
                    .arg(source)
                    .arg(format!("/toolcache/cargo/{directory}"));
            }
        }
    }
    if let Some(rustup_home) = &runner.rustup_home {
        process
            .arg("--ro-bind")
            .arg(rustup_home)
            .arg("/toolchain/rustup");
        process.args(["--setenv", "RUSTUP_HOME", "/toolchain/rustup"]);
    }
    path_entries.extend(["/usr/local/bin", "/usr/bin", "/bin"]);
    path_entries.join(":")
}

struct PnpmScratch {
    directory: tempfile::TempDir,
    source_files: PathBuf,
    source_cache: Option<PathBuf>,
}

fn configure_invocation(
    process: &mut Command,
    command: &PreparedCommand,
    path_value: String,
    pnpm_with_current: bool,
) {
    let sandbox_cwd = if command.cwd == "." {
        "/workspace".to_owned()
    } else {
        format!("/workspace/{}", command.cwd)
    };
    process
        .args(["--setenv", "PATH"])
        .arg(path_value)
        .args([
            "--setenv",
            "HOME",
            "/tmp/home",
            "--setenv",
            "TMPDIR",
            "/tmp",
            "--setenv",
            "CARGO_HOME",
            "/toolcache/cargo",
            "--setenv",
            "CI",
            "1",
            "--setenv",
            "TERM",
            "dumb",
            "--setenv",
            "LANG",
            "C.UTF-8",
            "--chdir",
        ])
        .arg(sandbox_cwd)
        .arg("--")
        .arg(&command.program);
    if command.program == "pnpm" && pnpm_with_current {
        process.args(["with", "current"]);
    }
    process.args(&command.args);
}

#[derive(Clone, Debug)]
struct Mask {
    kind: MaskKind,
    destination: PathBuf,
}

#[derive(Clone, Copy, Debug)]
enum MaskKind {
    File,
    Directory,
}

fn protected_masks(root: &Path) -> io::Result<Vec<Mask>> {
    let mut masks = Vec::new();
    let mut seen = HashSet::new();
    for entry in WalkBuilder::new(root)
        .hidden(false)
        .standard_filters(false)
        .follow_links(false)
        .build()
    {
        let entry = entry.map_err(|error| invalid(&error.to_string()))?;
        if entry.depth() == 0 {
            continue;
        }
        let relative = path::relative(root, entry.path());
        let is_git = entry.file_name() == ".git";
        if relative != ".git"
            && Path::new(&relative)
                .components()
                .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        if !is_git && !path::protected(&relative) {
            continue;
        }
        if seen.insert(relative.clone()) {
            masks.push(Mask {
                kind: if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                    MaskKind::Directory
                } else {
                    MaskKind::File
                },
                destination: PathBuf::from("/workspace").join(relative),
            });
        }
        if masks.len() > MAX_MASKS {
            return Err(invalid(
                "workspace has too many protected paths to sandbox safely",
            ));
        }
    }
    Ok(masks)
}

struct CapturedOutput {
    head: Vec<u8>,
    tail: Vec<u8>,
    total: usize,
    truncated: bool,
}

impl CapturedOutput {
    const fn new() -> Self {
        Self {
            head: Vec::new(),
            tail: Vec::new(),
            total: 0,
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        const HEAD_BYTES: usize = MAX_CAPTURE_BYTES / 4;
        const TAIL_BYTES: usize = MAX_CAPTURE_BYTES - HEAD_BYTES;
        self.total = self.total.saturating_add(bytes.len());
        let head_remaining = HEAD_BYTES.saturating_sub(self.head.len());
        let head_count = head_remaining.min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_count]);
        self.tail.extend_from_slice(&bytes[head_count..]);
        if self.tail.len() > TAIL_BYTES {
            self.tail.drain(..self.tail.len() - TAIL_BYTES);
        }
        self.truncated = self.total > MAX_CAPTURE_BYTES;
    }

    fn text(self) -> String {
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if self.truncated {
            text.push_str("\n… output truncated …\n");
        }
        text.push_str(&String::from_utf8_lossy(&self.tail));
        text
    }
}

async fn read_output(
    mut reader: impl AsyncRead + Unpin,
    stream: WorkspaceCommandOutputStream,
    sender: async_channel::Sender<Result<WorkspaceCommandEvent, WorkspaceCommandError>>,
) -> io::Result<CapturedOutput> {
    let mut capture = CapturedOutput::new();
    let mut ui_bytes = 0;
    let mut buffer = vec![0_u8; 8 * 1024];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        capture.push(&buffer[..count]);
        if ui_bytes < MAX_UI_BYTES && !sender.is_closed() {
            let available = (MAX_UI_BYTES - ui_bytes).min(count);
            ui_bytes += available;
            let chunk = String::from_utf8_lossy(&buffer[..available]).into_owned();
            let _ = sender
                .send(Ok(WorkspaceCommandEvent::Output { stream, chunk }))
                .await;
        }
    }
    Ok(capture)
}

fn probe(executable: &Path) -> io::Result<()> {
    let status = std::process::Command::new(executable)
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--ro-bind-try",
            "/usr",
            "/usr",
            "--ro-bind-try",
            "/bin",
            "/bin",
            "--ro-bind-try",
            "/lib",
            "/lib",
            "--ro-bind-try",
            "/lib64",
            "/lib64",
            "--ro-bind-try",
            "/etc",
            "/etc",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "/usr/bin/true",
        ])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("bubblewrap self-check failed"))
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn executable_root(name: &str) -> Option<PathBuf> {
    let executable = find_executable(name)?;
    let executable = executable.canonicalize().ok()?;
    let bin = executable.parent()?;
    (bin.file_name()? == "bin")
        .then(|| bin.parent().map(Path::to_path_buf))
        .flatten()
        .filter(|root| root.is_dir())
}

fn executable_major_version(name: &str) -> Option<u64> {
    let executable = find_executable(name)?;
    let output = std::process::Command::new(executable)
        .arg("--version")
        .current_dir(std::env::temp_dir())
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn pnpm_root() -> Option<PathBuf> {
    std::env::var_os("PNPM_HOME")
        .map(PathBuf::from)
        .and_then(|path| {
            let root = if path.file_name().is_some_and(|name| name == "bin") {
                path.parent().map(Path::to_path_buf)?
            } else {
                path
            };
            root.is_dir().then_some(root)
        })
        .or_else(|| executable_root("pnpm"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests;
