//! The reads that differ per operating system.
//!
//! **Every `#[cfg]` in the scan lives in this file.** Each parser is separated
//! from the subprocess that feeds it, so the parsing is tested on any machine
//! and only the invocation is platform-dependent. The parsers are `pub` for the
//! same reason as those in [`super::os`]: a Linux-only parser is unreachable,
//! and so dead code, in a Windows build.
//!
//! **Verified on Windows only.** The machine this was written on has no Linux
//! and no macOS; the `#[cfg(unix)]` invocations below have never been run.

use crate::snapshot::{GpuInfo, SystemSnapshot, VirtualizationInfo, WindowsInfo};

fn parse_bool(field: &str) -> Option<bool> {
    match field.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn vendor_of(model: &str) -> Option<String> {
    let lowered = model.to_ascii_lowercase();
    for (needle, vendor) in [
        ("nvidia", "NVIDIA"),
        ("amd", "AMD"),
        ("radeon", "AMD"),
        ("intel", "Intel"),
        ("apple", "Apple"),
    ] {
        if lowered.contains(needle) {
            return Some(vendor.to_string());
        }
    }
    None
}

/// `<VirtualizationFirmwareEnabled>,<HypervisorPresent>`.
///
/// Windows reports the firmware flag as `False` whenever a hypervisor is
/// already running, because the flag describes what firmware exposes to the
/// host rather than whether virtualization works. Trusting it unconditionally
/// would tell a machine already running Hyper-V to reboot into its BIOS, so a
/// present hypervisor is itself proof that virtualization is enabled.
pub fn parse_virtualization_csv(stdout: &str) -> VirtualizationInfo {
    let mut fields = stdout.trim().split(',');
    let firmware = fields.next().and_then(parse_bool);
    let hypervisor = fields.next().and_then(parse_bool);

    let enabled = match (firmware, hypervisor) {
        (_, Some(true)) => Some(true),
        (Some(flag), _) => Some(flag),
        (None, _) => None,
    };

    VirtualizationInfo {
        // On Windows the firmware flag only exists on a CPU that supports it,
        // and a running hypervisor proves support outright.
        supported: enabled.or(firmware),
        enabled,
        hypervisor_present: hypervisor,
    }
}

/// `vmx` (Intel) or `svm` (AMD) in `/proc/cpuinfo` flags.
pub fn parse_cpuinfo_flags(contents: &str) -> VirtualizationInfo {
    let Some(line) = contents
        .lines()
        .find(|line| line.trim_start().starts_with("flags"))
    else {
        return VirtualizationInfo::default();
    };
    let supported = line
        .split_whitespace()
        .any(|flag| flag == "vmx" || flag == "svm");

    VirtualizationInfo {
        supported: Some(supported),
        // A flag present in `/proc/cpuinfo` means the kernel can see the
        // feature, which on Linux means firmware has already enabled it.
        enabled: Some(supported),
        hypervisor_present: None,
    }
}

pub fn parse_wsl_status(stdout: &str) -> WindowsInfo {
    let value_after = |label: &str| -> Option<String> {
        stdout.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.trim().eq_ignore_ascii_case(label))
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    };

    let version = value_after("Default Version").and_then(|value| value.parse().ok());
    WindowsInfo {
        wsl_present: !stdout.trim().is_empty(),
        wsl_version: version,
        default_distro: value_after("Default Distribution"),
    }
}

pub fn parse_gpu_lines(stdout: &str) -> Vec<GpuInfo> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|model| GpuInfo {
            vendor: vendor_of(model),
            model: Some(model.to_string()),
        })
        .collect()
}

/// How long a probe subprocess may run before it is treated as absent.
///
/// Short on purpose. These answers decorate a snapshot; they must never decide
/// whether the window opens. `wsl --status` in particular is known to hang
/// indefinitely after a reboot, with no network, or when the WSL service is
/// wedged.
const SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// How a Windows probe talks to PowerShell without a console window.
///
/// `powershell.exe` rather than `powershell`: the latter can resolve to a Store
/// alias that allocates a visible console even when `CREATE_NO_WINDOW` is set.
/// `-WindowStyle Hidden` is the belt to that flag's braces — some hosts honour
/// one and not the other.
#[allow(dead_code)] // Used from the `#[cfg(windows)]` blocks below, and by the tests.
fn hidden_powershell(script: &str) -> (&'static str, Vec<String>) {
    (
        "powershell.exe",
        vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-WindowStyle".to_string(),
            "Hidden".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ],
    )
}

/// Run a command and return its stdout, or `None` if it could not be run.
///
/// Never propagates a failure: a machine where PowerShell is unavailable still
/// gets a snapshot, with these fields left unknown.
#[allow(dead_code)] // Used from the `#[cfg]` blocks below, one platform at a time.
fn output_of(program: &str, args: &[&str]) -> Option<String> {
    output_of_timed(program, args, SUBPROCESS_TIMEOUT)
}

#[allow(dead_code)] // Windows enrich; the timeout tests call `output_of` directly.
fn output_of_args(program: &str, args: &[String]) -> Option<String> {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    output_of(program, &borrowed)
}

/// Run a command, but stop waiting when `timeout` elapses.
///
/// Killing the child is load-bearing: without it a hung `wsl` or PowerShell
/// keeps running after we have moved on, and the next start can pile another
/// one on top.
///
/// stdout is drained on its own thread rather than after the exit. A pipe holds
/// only a buffer's worth, so a child that writes more than that blocks on the
/// write until somebody reads — and if the only reader waits for the exit
/// first, the two wait for each other until the deadline kills the child. A
/// machine with several video controllers is enough to reach that, and the
/// answer would be lost as a timeout rather than merely arriving slowly.
fn output_of_timed(program: &str, args: &[&str], timeout: std::time::Duration) -> Option<String> {
    use std::io::Read;

    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    crate::process::hide_console(&mut command);

    let mut child = command.spawn().ok()?;
    let pipe = child.stdout.take();
    let draining = std::thread::spawn(move || {
        let mut stdout = String::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_string(&mut stdout);
        }
        stdout
    });

    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // The child is gone, so its end of the pipe is closed and the
                // reader is at EOF or about to be. Joining cannot outlast it.
                let stdout = draining.join().unwrap_or_default();
                return status.success().then_some(stdout);
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Mark which operating system this is, without spawning anything.
///
/// File reads are fine: `/proc/cpuinfo` and `/etc/os-release` cannot hang the
/// way `wsl --status` can. Windows identity is recorded as "this is Windows"
/// with WSL unknown — filling WSL in is [`enrich`]'s job.
pub(crate) fn identify(snapshot: &mut SystemSnapshot) {
    #[cfg(windows)]
    {
        snapshot.windows = Some(WindowsInfo::default());
    }

    #[cfg(unix)]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/cpuinfo") {
            snapshot.virtualization = parse_cpuinfo_flags(&contents);
        }
        snapshot.linux = Some(
            std::fs::read_to_string("/etc/os-release")
                .map(|contents| super::os::parse_os_release(&contents))
                .unwrap_or_default(),
        );
    }
}

/// Fill in the facts that need a subprocess. Failures leave fields unknown.
///
/// Not called before the window opens. The commands below hang after a reboot
/// or when WSL is wedged, and a snapshot that waited for them was a snapshot
/// that kept the window closed.
pub(crate) fn enrich(snapshot: &mut SystemSnapshot) {
    #[cfg(windows)]
    {
        // Independent queries, so they run together. Sequential they cost up
        // to four timeouts; together they cost one.
        let virtualization = std::thread::spawn(|| {
            let (program, args) = hidden_powershell(
                "$c = Get-CimInstance Win32_ComputerSystem; \
                 $p = Get-CimInstance Win32_Processor | Select-Object -First 1; \
                 \"$($p.VirtualizationFirmwareEnabled),$($c.HypervisorPresent)\"",
            );
            output_of_args(program, &args)
        });
        let caption = std::thread::spawn(|| {
            let (program, args) =
                hidden_powershell("(Get-CimInstance Win32_OperatingSystem).Caption");
            output_of_args(program, &args)
        });
        let gpus = std::thread::spawn(|| {
            let (program, args) = hidden_powershell(
                "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
            );
            output_of_args(program, &args)
        });
        let wsl = std::thread::spawn(|| output_of("wsl.exe", &["--status"]));

        if let Ok(Some(stdout)) = virtualization.join() {
            snapshot.virtualization = parse_virtualization_csv(&stdout);
        }
        if let Ok(Some(stdout)) = caption.join() {
            let caption = stdout.trim().to_string();
            snapshot.os.edition = (!caption.is_empty()).then_some(caption);
        }
        if let Ok(Some(stdout)) = gpus.join() {
            snapshot.gpus = parse_gpu_lines(&stdout);
        }
        snapshot.windows = Some(
            wsl.join()
                .ok()
                .flatten()
                .map(|stdout| parse_wsl_status(&stdout))
                .unwrap_or_default(),
        );
    }

    #[cfg(unix)]
    {
        if let Some(stdout) = output_of("sh", &["-c", "lspci | grep -i vga"]) {
            snapshot.gpus = parse_gpu_lines(&stdout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_virtualization_output_is_parsed() {
        // `VirtualizationFirmwareEnabled,HypervisorPresent` from Win32_Processor
        // and Win32_ComputerSystem, emitted as one CSV line.
        let info = parse_virtualization_csv("True,False");
        assert_eq!(info.supported, Some(true));
        assert_eq!(info.enabled, Some(true));
        assert_eq!(info.hypervisor_present, Some(false));
    }

    #[test]
    fn virtualization_supported_but_disabled_is_distinguished_from_absent() {
        // These two produce completely different advice: one is a reboot into
        // firmware, the other has no fix on this machine at all.
        let disabled = parse_virtualization_csv("False,False");
        assert_eq!(disabled.enabled, Some(false));

        // When a hypervisor is already running, Windows reports the firmware
        // flag as False even though virtualization plainly works. Trusting it
        // would tell a working machine to reboot into its BIOS.
        let running = parse_virtualization_csv("False,True");
        assert_eq!(running.enabled, Some(true));
        assert_eq!(running.hypervisor_present, Some(true));
    }

    #[test]
    fn unreadable_virtualization_output_is_unknown_rather_than_false() {
        let info = parse_virtualization_csv("");
        assert_eq!(info.enabled, None);
        assert_eq!(parse_virtualization_csv("nonsense").enabled, None);
    }

    #[test]
    fn cpuinfo_flags_reveal_virtualization_support() {
        let intel = parse_cpuinfo_flags("flags\t: fpu vme de pse vmx est tm2\n");
        assert_eq!(intel.supported, Some(true));

        let amd = parse_cpuinfo_flags("flags\t: fpu vme de pse svm nx\n");
        assert_eq!(amd.supported, Some(true));

        let neither = parse_cpuinfo_flags("flags\t: fpu vme de pse\n");
        assert_eq!(neither.supported, Some(false));

        assert_eq!(parse_cpuinfo_flags("").supported, None);
    }

    #[test]
    fn wsl_status_reports_its_default_version() {
        let status = parse_wsl_status("Default Version: 2\nDefault Distribution: Ubuntu\n");
        assert!(status.wsl_present);
        assert_eq!(status.wsl_version, Some(2));
        assert_eq!(status.default_distro.as_deref(), Some("Ubuntu"));
    }

    #[test]
    fn absent_wsl_is_reported_as_absent() {
        let status = parse_wsl_status("");
        assert!(!status.wsl_present);
        assert_eq!(status.wsl_version, None);
    }

    #[test]
    fn gpu_lines_become_entries_and_blanks_are_dropped() {
        let gpus = parse_gpu_lines("NVIDIA GeForce RTX 4070\n\nIntel(R) UHD Graphics\n");
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].vendor.as_deref(), Some("NVIDIA"));
        assert_eq!(gpus[0].model.as_deref(), Some("NVIDIA GeForce RTX 4070"));
        assert_eq!(gpus[1].vendor.as_deref(), Some("Intel"));
        assert!(parse_gpu_lines("").is_empty());
    }

    #[test]
    fn a_windows_probe_asks_powershell_to_stay_hidden() {
        let (program, args) = hidden_powershell("Get-Date");
        assert_eq!(
            program, "powershell.exe",
            "the `powershell` alias can open a visible console"
        );
        assert!(
            args.windows(2).any(
                |pair| pair.first().map(String::as_str) == Some("-WindowStyle")
                    && pair.get(1).map(String::as_str) == Some("Hidden")
            ),
            "PowerShell was started without -WindowStyle Hidden: {args:?}"
        );
    }

    #[test]
    fn a_command_that_finishes_in_time_still_returns_its_output() {
        let result = if cfg!(windows) {
            output_of_timed(
                "cmd",
                &["/C", "echo hello"],
                std::time::Duration::from_secs(2),
            )
        } else {
            output_of_timed("echo", &["hello"], std::time::Duration::from_secs(2))
        };
        let stdout = result.expect("a command that finished should return its output");
        assert!(
            stdout.to_ascii_lowercase().contains("hello"),
            "got {stdout:?}"
        );
    }

    #[test]
    fn a_hung_command_is_abandoned_instead_of_stalling_the_caller() {
        // The launch hang: `wsl --status` and a CIM query that never returns
        // used to block `Runtime::start` until the process was killed. A probe
        // that cannot answer in time must come back empty, not wait it out.
        let started = std::time::Instant::now();
        let result = if cfg!(windows) {
            output_of_timed(
                "powershell",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 8",
                ],
                std::time::Duration::from_millis(400),
            )
        } else {
            output_of_timed("sleep", &["8"], std::time::Duration::from_millis(400))
        };
        let elapsed = started.elapsed();
        assert!(
            result.is_none(),
            "a command that outlived its budget still returned output"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "waited {elapsed:?} for a command that should have been killed at 400ms"
        );
    }
}
