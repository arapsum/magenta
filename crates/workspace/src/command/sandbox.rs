use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use async_process::Command;
use ignore::WalkBuilder;

use super::{BubblewrapCommandRunner, Mask, MaskKind, PnpmScratch, PreparedCommand, invalid};

pub(super) fn configure_sandbox(
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

pub(super) fn protected_masks(root: &Path) -> std::io::Result<Vec<Mask>> {
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
        let relative = super::path::relative(root, entry.path());
        let is_git = entry.file_name() == ".git";
        if relative != ".git"
            && Path::new(&relative)
                .components()
                .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        if !is_git && !super::path::protected(&relative) {
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
        if masks.len() > super::MAX_MASKS {
            return Err(invalid(
                "workspace has too many protected paths to sandbox safely",
            ));
        }
    }
    Ok(masks)
}
