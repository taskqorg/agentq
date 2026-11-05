//! Host resource samples (agentq.md § 6 Resource samples): CPU, memory and local disks on Linux, macOS, Windows and
//! WSL, one `agentq.resource` span per 300 s bucket through the telemetry queue.

use ring::{digest, hmac, rand::SecureRandom};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::time::Instant;

pub const EVERY: u64 = 300;
const WARMUP_MS: u64 = 10_000;
const STEP_MS: i128 = 2_000;

/// CPU ticks: (busy, total) since boot.
type Ticks = (u64, u64);

/// One filesystem as read: mount point and fstype stay local; stat is (total, free, used) bytes.
pub struct Disk {
    mount: String,
    fstype: String,
    stat: Result<(u64, u64, u64), String>,
}

pub struct Reading {
    cpu: Result<Ticks, String>,
    /// (total, available) bytes
    memory: Result<(u64, u64), String>,
    disks: Result<Vec<Disk>, String>,
}

pub fn read() -> Reading {
    Reading { cpu: os::cpu(), memory: os::memory(), disks: os::disks() }
}

pub struct Sampler {
    key: hmac::Key,
    bucket: u64,
    /// previous read: monotonic time, wall ns, CPU ticks
    prev: (Instant, u128, Result<Ticks, String>),
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `data/resource-key`: 32 random bytes as hex, created on first use, never exported.
pub(crate) fn key(data: &Path) -> hmac::Key {
    let p = data.join("resource-key");
    let k = match std::fs::read_to_string(&p) {
        Ok(s) if s.trim().len() == 64 => s.trim().to_string(),
        _ => {
            let mut b = [0u8; 32];
            let _ = ring::rand::SystemRandom::new().fill(&mut b);
            let k = hex(&b);
            let _ = std::fs::create_dir_all(data).map_err(|e| e.to_string()).and_then(|_| coreq::write_atomic(&p, &k));
            k
        }
    };
    hmac::Key::new(hmac::HMAC_SHA256, k.as_bytes())
}

/// Same installation and bucket, same ID: a resend or a second sampler replaces the trace.
pub fn event_id(installation: &str, bucket: u64) -> String {
    hex(digest::digest(&digest::SHA256, format!("{installation}:resource:{bucket}").as_bytes()).as_ref())[..32].to_string()
}

impl Sampler {
    /// Reads the CPU baseline; the first sample comes in the next bucket.
    pub fn new(data: &Path) -> Sampler {
        let ns = crate::telemetry::now_ns();
        Sampler { key: key(data), bucket: (ns / 1_000_000_000) as u64 / EVERY, prev: (Instant::now(), ns, os::cpu()) }
    }

    /// Queue one sample when a new bucket started; returns its event ID.
    pub fn tick(&mut self, data: &Path) -> Option<String> {
        let ns = crate::telemetry::now_ns();
        let bucket = (ns / 1_000_000_000) as u64 / EVERY;
        if bucket == self.bucket {
            return None;
        }
        self.bucket = bucket;
        let mut a = self.sample(read(), Instant::now(), ns);
        a.insert("agentq.event_id".into(), json!(event_id(&crate::telemetry::installation(data), bucket)));
        a.insert("agentq.ids_unknown".into(), json!("a resource sample has no session"));
        let id = crate::telemetry::emit_at(data, "agentq.resource", "sample", Value::Object(a), ns)?;
        // the newest sample outlives its queue file for the enrollment snapshot (`connect.rs`)
        let _ = std::fs::copy(crate::telemetry::queue(data).join(format!("{}-{id}.json", ns / 1_000_000_000)), data.join("resource.json"));
        Some(id)
    }

    /// The `agentq.resource.*` attributes of one reading; it becomes the next CPU baseline.
    pub(crate) fn sample(&mut self, r: Reading, mono: Instant, ns: u128) -> Map<String, Value> {
        let mut a = Map::new();
        let mut put = |k: &str, v: Value| a.insert(format!("agentq.resource.{k}"), v);
        let (mono0, ns0, cpu0) = std::mem::replace(&mut self.prev, (mono, ns, r.cpu.clone()));
        let ms = mono.saturating_duration_since(mono0).as_millis() as u64;
        let step = (ns as i128 - ns0 as i128) / 1_000_000 - ms as i128;
        if step.abs() > STEP_MS {
            put("clock_step_ms", json!(step as i64));
        }
        match (cpu0, r.cpu) {
            (_, Err(e)) => put("cpu_unknown", json!(e)),
            (Err(_), _) => put("cpu_unknown", json!("warmup")),
            _ if ms < WARMUP_MS => put("cpu_unknown", json!("warmup")),
            (Ok((b0, t0)), Ok((b1, t1))) if b1 < b0 || t1 <= t0 || t1 - t0 < b1 - b0 => put("cpu_unknown", json!("counter reset")),
            (Ok((b0, t0)), Ok((b1, t1))) => {
                put("cpu_percent", json!(((b1 - b0) as f64 * 1000.0 / (t1 - t0) as f64).round() / 10.0));
                put("cpu_interval_ms", json!(ms))
            }
        };
        match r.memory {
            Ok((t, av)) if t > 0 && av <= t => {
                put("memory_total_bytes", json!(t));
                put("memory_available_bytes", json!(av));
                put("memory_used_bytes", json!(t - av))
            }
            Ok(_) => put("memory_unknown", json!("available exceeds total")),
            Err(e) => put("memory_unknown", json!(e)),
        };
        match r.disks {
            Ok(d) if d.is_empty() => put("disk_unknown", json!("no local filesystem")),
            Ok(d) => {
                let list: Vec<Value> = d.iter().map(|d| self.disk(d)).collect();
                put("disks", json!(Value::Array(list).to_string()))
            }
            Err(e) => put("disk_unknown", json!(e)),
        };
        a
    }

    fn disk(&self, d: &Disk) -> Value {
        let id = hex(hmac::sign(&self.key, format!("{}\0{}", d.mount, d.fstype).as_bytes()).as_ref())[..16].to_string();
        let role = if d.mount == os::root() { "root" } else { "other" };
        match &d.stat {
            Ok((t, f, u)) => json!({"id": id, "role": role, "fstype": d.fstype, "used_bytes": u, "free_bytes": f, "total_bytes": t}),
            Err(e) => json!({"id": id, "role": role, "fstype": d.fstype, "used_bytes": null, "free_bytes": null, "total_bytes": null, "unknown": e}),
        }
    }
}

/// `/proc/stat` first line: busy = user nice system irq softirq steal; total adds idle and iowait.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_stat(s: &str) -> Result<Ticks, String> {
    let f: Vec<u64> = s.lines().find(|l| l.starts_with("cpu ")).ok_or("/proc/stat: no cpu line")?.split_whitespace().skip(1).map_while(|x| x.parse().ok()).collect();
    let [user, nice, system, idle, iowait, irq, softirq, steal, ..] = f[..] else { return Err("/proc/stat: short cpu line".into()) };
    let busy = user + nice + system + irq + softirq + steal;
    Ok((busy, busy + idle + iowait))
}

/// `/proc/meminfo` `MemTotal` and `MemAvailable` in bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_meminfo(s: &str) -> Result<(u64, u64), String> {
    let kb = |k: &str| {
        s.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix(':')?.trim().strip_suffix(" kB")?.trim().parse::<u64>().ok()).map(|v| v * 1024).ok_or(format!("/proc/meminfo: no {k}"))
    };
    Ok((kb("MemTotal")?, kb("MemAvailable")?))
}

/// WSL 1 has no VM: its `/proc` mirrors the Windows host, which its own installation already reports.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn wsl1(osrelease: &str) -> bool {
    osrelease.contains("Microsoft")
}

/// Windows `GetSystemTimes` (idle, kernel, user) in 100 ns units; kernel time includes idle.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn system_times(idle: u64, kernel: u64, user: u64) -> Ticks {
    let busy = kernel.saturating_sub(idle) + user;
    (busy, kernel + user)
}

/// `GetLogicalDriveStringsW` buffer: NUL-separated roots such as `C:\`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_drives(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0).filter(|r| !r.is_empty()).map(String::from_utf16_lossy).collect()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const LOCAL_FS: &[&str] = &["ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "zfs", "bcachefs", "jfs", "vfat", "exfat", "ntfs3"];

/// `/proc/self/mountinfo`: (mount point, fstype) of local block filesystems, one per device, shortest mount first.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_mountinfo(s: &str) -> Vec<(String, String)> {
    let unescape = |p: &str| {
        let mut out = String::new();
        let mut rest = p;
        while let Some(i) = rest.find('\\') {
            out.push_str(&rest[..i]);
            match rest.get(i + 1..i + 4).and_then(|o| u8::from_str_radix(o, 8).ok()) {
                Some(c) => (out.push(c as char), rest = &rest[i + 4..]),
                None => (out.push('\\'), rest = &rest[i + 1..]),
            };
        }
        out + rest
    };
    let mut v: Vec<(usize, String, String, String)> = s
        .lines()
        .filter_map(|l| {
            let (pre, post) = l.split_once(" - ")?;
            let pre: Vec<&str> = pre.split(' ').collect();
            let fstype = post.split(' ').next()?;
            let mount = unescape(pre.get(4)?);
            let dev = pre.get(2)?.to_string();
            LOCAL_FS.contains(&fstype).then(|| (mount.len(), mount, fstype.to_string(), dev))
        })
        .collect();
    v.sort();
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|e| seen.insert(e.3.clone())).map(|e| (e.1, e.2)).collect()
}

#[cfg(target_os = "linux")]
mod os {
    use super::*;

    fn file(p: &str) -> Result<String, String> {
        std::fs::read_to_string(p).map_err(|e| format!("{p}: {}", e.kind()))
    }

    const WSL1: &str = "wsl1 mirrors the Windows host";

    pub fn root() -> &'static str {
        "/"
    }

    fn mirrored() -> bool {
        crate::telemetry::runtime() == "wsl" && file("/proc/sys/kernel/osrelease").is_ok_and(|s| wsl1(&s))
    }

    pub fn cpu() -> Result<Ticks, String> {
        if mirrored() {
            return Err(WSL1.into());
        }
        parse_stat(&file("/proc/stat")?)
    }

    pub fn memory() -> Result<(u64, u64), String> {
        if mirrored() {
            return Err(WSL1.into());
        }
        parse_meminfo(&file("/proc/meminfo")?)
    }

    pub fn disks() -> Result<Vec<Disk>, String> {
        Ok(parse_mountinfo(&file("/proc/self/mountinfo")?).into_iter().map(|(mount, fstype)| Disk { stat: stat(&mount), mount, fstype }).collect())
    }

    fn stat(mount: &str) -> Result<(u64, u64, u64), String> {
        let p = std::ffi::CString::new(mount).map_err(|_| "mount point holds NUL".to_string())?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(p.as_ptr(), &mut s) } != 0 {
            return Err(format!("statvfs: {}", std::io::Error::last_os_error().kind()));
        }
        let b = s.f_frsize as u64;
        Ok((s.f_blocks as u64 * b, s.f_bavail as u64 * b, (s.f_blocks as u64).saturating_sub(s.f_bfree as u64) * b))
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
mod os {
    use super::*;
    use std::ffi::CStr;

    // ponytail: one host port per process, never released; mach2 crate if more Mach calls appear.
    fn host() -> libc::mach_port_t {
        static H: std::sync::OnceLock<libc::mach_port_t> = std::sync::OnceLock::new();
        *H.get_or_init(|| unsafe { libc::mach_host_self() })
    }

    pub fn root() -> &'static str {
        "/"
    }

    pub fn cpu() -> Result<Ticks, String> {
        let mut i: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
        let mut n = libc::HOST_CPU_LOAD_INFO_COUNT;
        let r = unsafe { libc::host_statistics(host(), libc::HOST_CPU_LOAD_INFO, &mut i as *mut _ as *mut i32, &mut n) };
        if r != 0 {
            return Err(format!("host_statistics: kern {r}"));
        }
        let t = i.cpu_ticks.map(u64::from);
        let busy = t[libc::CPU_STATE_USER as usize] + t[libc::CPU_STATE_SYSTEM as usize] + t[libc::CPU_STATE_NICE as usize];
        Ok((busy, busy + t[libc::CPU_STATE_IDLE as usize]))
    }

    pub fn memory() -> Result<(u64, u64), String> {
        let mut total = 0u64;
        let mut len = std::mem::size_of::<u64>();
        if unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), &mut total as *mut _ as *mut libc::c_void, &mut len, std::ptr::null_mut(), 0) } != 0 {
            return Err(format!("hw.memsize: {}", std::io::Error::last_os_error().kind()));
        }
        let mut v: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        let mut n = libc::HOST_VM_INFO64_COUNT;
        let r = unsafe { libc::host_statistics64(host(), libc::HOST_VM_INFO64, &mut v as *mut _ as *mut i32, &mut n) };
        if r != 0 {
            return Err(format!("host_statistics64: kern {r}"));
        }
        let page = unsafe { libc::vm_page_size } as u64;
        let used = (v.internal_page_count.saturating_sub(v.purgeable_count) as u64 + v.wire_count as u64 + v.compressor_page_count as u64) * page;
        Ok((total, total.saturating_sub(used)))
    }

    pub fn disks() -> Result<Vec<Disk>, String> {
        let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
        if n < 0 {
            return Err(format!("getfsstat: {}", std::io::Error::last_os_error().kind()));
        }
        let mut v: Vec<libc::statfs> = vec![unsafe { std::mem::zeroed() }; n as usize + 4];
        let size = (v.len() * std::mem::size_of::<libc::statfs>()) as libc::c_int;
        let n = unsafe { libc::getfsstat(v.as_mut_ptr(), size, libc::MNT_NOWAIT) };
        if n < 0 {
            return Err(format!("getfsstat: {}", std::io::Error::last_os_error().kind()));
        }
        v.truncate(n as usize);
        Ok(v.iter()
            .filter(|s| s.f_flags & libc::MNT_LOCAL as u32 != 0 && s.f_flags & libc::MNT_DONTBROWSE as u32 == 0)
            .map(|s| {
                let mount = unsafe { CStr::from_ptr(s.f_mntonname.as_ptr()) }.to_string_lossy().into_owned();
                let fstype = unsafe { CStr::from_ptr(s.f_fstypename.as_ptr()) }.to_string_lossy().into_owned();
                Disk { stat: stat(&mount), mount, fstype }
            })
            .collect())
    }

    /// Fresh numbers per mount; the getfsstat list is `MNT_NOWAIT` cache.
    fn stat(mount: &str) -> Result<(u64, u64, u64), String> {
        let p = std::ffi::CString::new(mount).map_err(|_| "mount point holds NUL".to_string())?;
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(p.as_ptr(), &mut s) } != 0 {
            return Err(format!("statfs: {}", std::io::Error::last_os_error().kind()));
        }
        let b = s.f_bsize as u64;
        Ok((s.f_blocks * b, s.f_bavail * b, s.f_blocks.saturating_sub(s.f_bfree) * b))
    }
}

#[cfg(windows)]
mod os {
    use super::*;

    #[repr(C)]
    struct MemoryStatusEx {
        len: u32,
        load: u32,
        total_phys: u64,
        avail_phys: u64,
        rest: [u64; 5],
    }

    const DRIVE_FIXED: u32 = 3;

    // ponytail: six kernel32 calls declared by hand; the windows-sys crate if more Win32 appears.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemTimes(idle: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn GlobalMemoryStatusEx(m: *mut MemoryStatusEx) -> i32;
        fn GetLogicalDriveStringsW(len: u32, buf: *mut u16) -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_len: u32, serial: *mut u32, max_len: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
        fn GetDiskFreeSpaceExW(root: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }

    fn err(call: &str) -> String {
        format!("{call}: {}", std::io::Error::last_os_error().kind())
    }

    /// The system drive, `C:\` unless `SystemDrive` says otherwise.
    pub fn root() -> &'static str {
        static R: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        R.get_or_init(|| format!("{}\\", std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_uppercase()))
    }

    pub fn cpu() -> Result<Ticks, String> {
        let (mut i, mut k, mut u) = (0u64, 0u64, 0u64);
        if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 {
            return Err(err("GetSystemTimes"));
        }
        Ok(system_times(i, k, u))
    }

    pub fn memory() -> Result<(u64, u64), String> {
        let mut m = MemoryStatusEx { len: std::mem::size_of::<MemoryStatusEx>() as u32, load: 0, total_phys: 0, avail_phys: 0, rest: [0; 5] };
        if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
            return Err(err("GlobalMemoryStatusEx"));
        }
        Ok((m.total_phys, m.avail_phys))
    }

    pub fn disks() -> Result<Vec<Disk>, String> {
        let mut buf = [0u16; 512];
        let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
        if n == 0 || n > buf.len() {
            return Err(err("GetLogicalDriveStringsW"));
        }
        Ok(parse_drives(&buf[..n])
            .into_iter()
            .filter_map(|mount| {
                let w: Vec<u16> = mount.encode_utf16().chain([0]).collect();
                (unsafe { GetDriveTypeW(w.as_ptr()) } == DRIVE_FIXED).then(|| Disk { fstype: fstype(&w), stat: stat(&w), mount: mount.to_uppercase() })
            })
            .collect())
    }

    fn fstype(root: &[u16]) -> String {
        let mut fs = [0u16; 32];
        let n = std::ptr::null_mut();
        if unsafe { GetVolumeInformationW(root.as_ptr(), n, 0, n.cast(), n.cast(), n.cast(), fs.as_mut_ptr(), fs.len() as u32) } == 0 {
            return "unknown".into();
        }
        parse_drives(&fs).into_iter().next().unwrap_or_else(|| "unknown".into())
    }

    fn stat(root: &[u16]) -> Result<(u64, u64, u64), String> {
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
            return Err(err("GetDiskFreeSpaceExW"));
        }
        Ok((total, avail, total.saturating_sub(free)))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod os {
    use super::*;

    const NO: &str = "no sampler on this OS";

    pub fn root() -> &'static str {
        "/"
    }

    pub fn cpu() -> Result<Ticks, String> {
        Err(NO.into())
    }

    pub fn memory() -> Result<(u64, u64), String> {
        Err(NO.into())
    }

    pub fn disks() -> Result<Vec<Disk>, String> {
        Err(NO.into())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    pub(crate) fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-res-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn disk(mount: &str, stat: Result<(u64, u64, u64), String>) -> Disk {
        Disk { mount: mount.into(), fstype: "ext4".into(), stat }
    }

    pub(crate) fn reading(cpu: Result<Ticks, String>) -> Reading {
        Reading { cpu, memory: Ok((8 << 30, 3 << 30)), disks: Ok(vec![disk(os::root(), Ok((100, 30, 60))), disk("/home/alice/Private Vault", Err("statvfs: permission denied".into()))]) }
    }

    pub(crate) fn sampler(d: &Path, cpu: Result<Ticks, String>, at: Instant) -> Sampler {
        Sampler { key: key(d), bucket: 0, prev: (at, 1_000_000_000_000, cpu) }
    }

    #[test]
    fn parse_linux_probes() {
        let stat = "cpu  100 5 50 800 20 3 2 10 7 0\ncpu0 1 2 3 4 5 6 7 8 9 0\nintr 1\n";
        assert_eq!(parse_stat(stat), Ok((170, 990)), "guest stays inside user");
        assert!(parse_stat("cpu  1 2 3\n").is_err_and(|e| e.contains("short")));
        assert!(parse_stat("intr 1\n").is_err());
        let mem = "MemTotal:       16303428 kB\nMemFree:          812344 kB\nMemAvailable:    9120000 kB\n";
        assert_eq!(parse_meminfo(mem), Ok((16303428 * 1024, 9120000 * 1024)));
        assert!(parse_meminfo("MemTotal: 1 kB\nMemFree: 1 kB\n").is_err_and(|e| e.contains("MemAvailable")), "old kernels say why");
        let mounts = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw - proc proc rw
24 22 0:5 / /dev rw - devtmpfs udev rw
25 22 259:1 / /boot/efi rw - vfat /dev/nvme0n1p1 rw
26 22 0:30 / /run/user/1000 rw - tmpfs tmpfs rw
27 22 259:2 /home /srv/bind rw - ext4 /dev/nvme0n1p2 rw
28 22 0:40 / /mnt/My\\040Disk rw - btrfs /dev/sda1 rw
29 22 0:40 /@snap /mnt/snap rw - btrfs /dev/sda1 rw
30 22 7:0 / /snap/core/1 ro - squashfs /dev/loop0 ro
31 22 0:50 / /mnt/nas rw - nfs4 nas:/x rw
32 22 0:51 / /home/u/.gvfs rw - fuse.gvfsd-fuse gvfsd rw
33 22 0:52 / /var/lib/docker/overlay2/x/merged rw - overlay overlay rw
";
        let got = parse_mountinfo(mounts);
        let want = [("/", "ext4"), ("/boot/efi", "vfat"), ("/mnt/snap", "btrfs")];
        assert_eq!(got, want.map(|(m, f)| (m.to_string(), f.to_string())), "local block only, one per device, shortest mount first");
    }

    /// WSL 2 keeps its own ext4 disks and drops Windows drives; WSL 1 is detected; Windows counters and drive lists.
    #[test]
    fn parse_wsl_and_windows_probes() {
        let wsl = "\
60 1 8:32 / / rw,relatime - ext4 /dev/sdc rw,discard
61 60 0:15 / /mnt/wsl rw - tmpfs none rw
62 60 0:33 / /usr/lib/wsl/drivers ro - 9p drivers ro,aname=drivers
63 60 8:32 /mnt/wslg/distro /mnt/wslg/distro rw - ext4 /dev/sdc rw
64 60 0:40 / /mnt/c rw,noatime - 9p C:\\134 rw,aname=drvfs;path=C:\\
65 60 0:41 / /mnt/d rw,noatime - drvfs D:\\134 rw
66 60 8:48 / /mnt/wsl/docker-desktop-data rw - ext4 /dev/sdd rw
";
        let got = parse_mountinfo(wsl);
        let want = [("/", "ext4"), ("/mnt/wsl/docker-desktop-data", "ext4")];
        assert_eq!(got, want.map(|(m, f)| (m.to_string(), f.to_string())), "no Windows drive counted twice");
        assert!(wsl1("4.4.0-19041-Microsoft\n"));
        assert!(!wsl1("5.15.153.1-microsoft-standard-WSL2\n"));
        assert!(!wsl1("6.8.0-45-generic\n"));
        // 100 ns units: kernel includes idle
        assert_eq!(system_times(700, 1000, 300), (600, 1300));
        let buf: Vec<u16> = "C:\\\0D:\\\0\0".encode_utf16().collect();
        assert_eq!(parse_drives(&buf), ["C:\\", "D:\\"]);
        assert!(parse_drives(&[0; 8]).is_empty());
    }

    /// A mount that appears or goes changes the entries of the next sample, never a zero; the others keep their IDs.
    #[test]
    fn mount_change() {
        let d = dir("mount");
        let t0 = Instant::now();
        let mut s = sampler(&d, Ok((0, 1)), t0);
        let disks = |s: &mut Sampler, list: Vec<Disk>, at: u64| -> Vec<Value> {
            let r = Reading { cpu: Ok((0, 1)), memory: Ok((1, 1)), disks: Ok(list) };
            let a = s.sample(r, t0 + Duration::from_secs(at), 1_000_000_000_000 + at as u128 * 1_000_000_000);
            serde_json::from_str::<Vec<Value>>(a["agentq.resource.disks"].as_str().unwrap()).unwrap()
        };
        let one = disks(&mut s, vec![disk("/", Ok((100, 30, 60)))], 300);
        let two = disks(&mut s, vec![disk("/", Ok((100, 30, 60))), disk("/mnt/usb", Ok((50, 50, 0)))], 600);
        let back = disks(&mut s, vec![disk("/", Ok((100, 20, 70)))], 900);
        assert_eq!((one.len(), two.len(), back.len()), (1, 2, 1));
        assert_eq!((one[0]["id"].clone(), back[0]["id"].clone()), (two[0]["id"].clone(), two[0]["id"].clone()));
        assert_ne!(two[1]["id"], two[0]["id"]);
        assert_eq!(back[0]["used_bytes"], json!(70));
        let _ = std::fs::remove_dir_all(d);
    }

    /// Values with units, unknown with reasons, no private path; restart and clock rules.
    #[test]
    fn sample_rules() {
        let d = dir("sample");
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(300);
        let mut s = sampler(&d, Ok((100, 1000)), t0);
        let a = s.sample(reading(Ok((350, 2000))), t1, 1_000_000_000_000 + 300_000_000_000);
        assert_eq!(a["agentq.resource.cpu_percent"], json!(25.0));
        assert_eq!(a["agentq.resource.cpu_interval_ms"], json!(300_000));
        assert_eq!(a["agentq.resource.memory_used_bytes"], json!(5u64 << 30));
        assert!(!a.contains_key("agentq.resource.clock_step_ms"));
        let disks: Value = serde_json::from_str(a["agentq.resource.disks"].as_str().unwrap()).unwrap();
        assert_eq!(disks[0]["role"], "root");
        assert_eq!((disks[0]["total_bytes"].clone(), disks[0]["free_bytes"].clone(), disks[0]["used_bytes"].clone()), (json!(100), json!(30), json!(60)));
        assert!(disks[1]["total_bytes"].is_null() && disks[1]["unknown"] == "statvfs: permission denied", "a failed stat is null, never zero");
        assert_eq!(disks[1]["id"].as_str().unwrap().len(), 16);
        assert!(!a.values().any(|v| v.to_string().contains("alice") || v.to_string().contains("Vault")), "no mount path exported");
        // the same key gives the same ID after a restart; another installation's key does not
        let again = sampler(&d, Ok((0, 1)), t0).disk(&disk("/home/alice/Private Vault", Ok((1, 1, 0))));
        assert_eq!(again["id"], disks[1]["id"]);
        let other = dir("other");
        assert_ne!(sampler(&other, Ok((0, 1)), t0).disk(&disk("/home/alice/Private Vault", Ok((1, 1, 0))))["id"], disks[1]["id"]);

        // a clock step forward by 1 h: noted, CPU still from the monotonic interval
        let a = s.sample(reading(Ok((400, 3000))), t1 + Duration::from_secs(300), 1_000_000_000_000 + 4200_000_000_000);
        assert_eq!(a["agentq.resource.clock_step_ms"], json!(3_600_000));
        assert_eq!(a["agentq.resource.cpu_percent"], json!(5.0));

        // counters backwards (macOS 32-bit wrap), probe failure, warmup after a restart
        let a = s.sample(reading(Ok((10, 3100))), t1 + Duration::from_secs(600), 1_000_000_000_000 + 4500_000_000_000);
        assert_eq!(a["agentq.resource.cpu_unknown"], "counter reset");
        assert!(!a.contains_key("agentq.resource.cpu_percent"));
        let a = s.sample(reading(Err("/proc/stat: permission denied".into())), t1 + Duration::from_secs(900), 1_000_000_000_000 + 4800_000_000_000);
        assert_eq!(a["agentq.resource.cpu_unknown"], "/proc/stat: permission denied");
        let a = s.sample(reading(Ok((20, 3200))), t1 + Duration::from_secs(1200), 1_000_000_000_000 + 5100_000_000_000);
        assert_eq!(a["agentq.resource.cpu_unknown"], "warmup", "no baseline after a failed read");
        let mut fresh = sampler(&d, Ok((0, 100)), t0);
        assert_eq!(fresh.sample(reading(Ok((50, 200))), t0 + Duration::from_secs(5), 1_000_005_000_000)["agentq.resource.cpu_unknown"], "warmup");

        let bad = Reading { cpu: Ok((0, 1)), memory: Err("hw.memsize: permission denied".into()), disks: Ok(vec![]) };
        let a = s.sample(bad, t1 + Duration::from_secs(1500), 0);
        assert_eq!(a["agentq.resource.memory_unknown"], "hw.memsize: permission denied");
        assert!(!a.contains_key("agentq.resource.memory_total_bytes"));
        assert_eq!(a["agentq.resource.disk_unknown"], "no local filesystem");
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(other);
    }

    /// One queue file and one trace per installation and bucket; doubles stay doubles on the wire.
    #[test]
    fn dedup_in_queue() {
        let d = dir("dedup");
        let id = event_id("inst", 7);
        assert_eq!(id, event_id("inst", 7));
        assert_ne!(id, event_id("inst", 8));
        assert_ne!(id, event_id("other", 7));
        for ns in [2_100_000_000_000u128, 2_130_000_000_000] {
            let attrs = json!({"agentq.event_id": id, "agentq.resource.cpu_percent": 12.0});
            assert_eq!(crate::telemetry::emit_at(&d, "agentq.resource", "sample", attrs, ns).as_deref(), Some(id.as_str()));
        }
        let files: Vec<_> = std::fs::read_dir(d.join("queue")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(files, [format!("2130-{id}.json")], "the later sample replaced the earlier one");
        let s: Value = serde_json::from_str(&std::fs::read_to_string(d.join("queue").join(&files[0])).unwrap()).unwrap();
        assert_eq!(s["traceId"], id);
        let cpu = s["attributes"].as_array().unwrap().iter().find(|a| a["key"] == "agentq.resource.cpu_percent").unwrap();
        assert_eq!(cpu["value"], json!({"doubleValue": 12.0}));
        let _ = std::fs::remove_dir_all(d);
    }

    /// The real probes on this host: plausible values and the overhead bound of § 6.
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn live_probe_overhead() {
        let start = Instant::now();
        let r = read();
        let took = start.elapsed();
        assert!(took < Duration::from_millis(50), "one sample took {took:?}");
        if std::fs::read_to_string("/proc/sys/kernel/osrelease").is_ok_and(|s| wsl1(&s)) {
            return assert_eq!(r.cpu, Err("wsl1 mirrors the Windows host".into()));
        }
        let (b, t) = r.cpu.unwrap();
        assert!(b <= t && t > 0);
        let (total, av) = r.memory.unwrap();
        assert!(total > 0 && av <= total, "{total} {av}");
        let disks = r.disks.unwrap();
        let root = disks.iter().find(|d| d.mount == os::root()).expect("root filesystem listed");
        let (t, f, u) = root.stat.clone().unwrap();
        assert!(t > 0 && f <= t && u <= t);
        eprintln!("resource sample: {took:?}; memory total {total} available {av}; root total {t} free {f} used {u}; {} filesystems", disks.len());
    }
}
