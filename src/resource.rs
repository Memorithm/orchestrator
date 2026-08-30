use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

const DEFAULT_MIN_AVAILABLE_MEMORY_MB: u64 = 4_096;
const DEFAULT_MIN_FREE_DISK_MB: u64 = 8_192;
const DEFAULT_MAX_LOAD_PER_CPU: f64 = 2.0;
const BYTES_PER_MIB: u128 = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourcePolicy {
    pub min_available_memory_mb: u64,
    pub min_free_disk_mb: u64,
    pub max_load_per_cpu: f64,
}

impl ResourcePolicy {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            min_available_memory_mb: parse_env_u64(
                "ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB",
                DEFAULT_MIN_AVAILABLE_MEMORY_MB,
            )?,
            min_free_disk_mb: parse_env_u64(
                "ORCHESTRATOR_MIN_FREE_DISK_MB",
                DEFAULT_MIN_FREE_DISK_MB,
            )?,
            max_load_per_cpu: parse_env_f64(
                "ORCHESTRATOR_MAX_LOAD_PER_CPU",
                DEFAULT_MAX_LOAD_PER_CPU,
            )?,
        })
    }

    pub fn evaluate(self, snapshot: HostResources) -> Admission {
        if self.min_available_memory_mb > 0
            && snapshot.available_memory_mb < self.min_available_memory_mb
        {
            return Admission::Deferred {
                snapshot,
                pressure: PressureKind::Memory,
                reason: format!(
                    "available memory {} MiB is below required {} MiB",
                    snapshot.available_memory_mb, self.min_available_memory_mb
                ),
            };
        }

        if self.min_free_disk_mb > 0 && snapshot.free_disk_mb < self.min_free_disk_mb {
            return Admission::Deferred {
                snapshot,
                pressure: PressureKind::Disk,
                reason: format!(
                    "free data-root disk {} MiB is below required {} MiB",
                    snapshot.free_disk_mb, self.min_free_disk_mb
                ),
            };
        }

        let normalized_load = snapshot.load_per_cpu();
        if self.max_load_per_cpu > 0.0 && normalized_load > self.max_load_per_cpu {
            return Admission::Deferred {
                snapshot,
                pressure: PressureKind::Load,
                reason: format!(
                    "1m load per CPU {:.2} exceeds configured {:.2}",
                    normalized_load, self.max_load_per_cpu
                ),
            };
        }

        Admission::Admitted(snapshot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureKind {
    Memory,
    Disk,
    Load,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostResources {
    pub available_memory_mb: u64,
    pub free_disk_mb: u64,
    pub load_one: f64,
    pub cpu_count: u32,
}

impl HostResources {
    pub fn load_per_cpu(self) -> f64 {
        self.load_one / f64::from(self.cpu_count)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    Admitted(HostResources),
    Deferred {
        snapshot: HostResources,
        pressure: PressureKind,
        reason: String,
    },
}

pub fn sample_linux(data_root: &Path) -> Result<HostResources, String> {
    let meminfo = fs::read_to_string("/proc/meminfo")
        .map_err(|error| format!("failed to read /proc/meminfo: {error}"))?;
    let loadavg = fs::read_to_string("/proc/loadavg")
        .map_err(|error| format!("failed to read /proc/loadavg: {error}"))?;
    let cpu_count = std::thread::available_parallelism()
        .map_err(|error| format!("failed to determine available CPU parallelism: {error}"))?;
    let cpu_count = u32::try_from(cpu_count.get())
        .map_err(|_| "available CPU count does not fit in u32".to_owned())?;

    Ok(HostResources {
        available_memory_mb: parse_mem_available_mb(&meminfo)?,
        free_disk_mb: sample_free_disk_mb(data_root)?,
        load_one: parse_load_one(&loadavg)?,
        cpu_count,
    })
}

fn sample_free_disk_mb(path: &Path) -> Result<u64, String> {
    let output = Command::new("stat")
        .args(["-f", "-c", "%a %S", "--"])
        .arg(path)
        .output()
        .map_err(|error| format!("failed to execute stat for {}: {error}", path.display()))?;

    if !output.status.success() {
        return Err(format!(
            "stat filesystem query failed for {} with {}: {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("invalid UTF-8 from stat for {}: {error}", path.display()))?;
    parse_statfs_available_mb(&stdout)
}

fn parse_statfs_available_mb(input: &str) -> Result<u64, String> {
    let fields = input.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2 {
        return Err(format!(
            "expected exactly two stat filesystem fields, got {}",
            fields.len()
        ));
    }

    let available_blocks = fields[0]
        .parse::<u128>()
        .map_err(|error| format!("invalid available filesystem blocks: {error}"))?;
    let block_size = fields[1]
        .parse::<u128>()
        .map_err(|error| format!("invalid filesystem block size: {error}"))?;
    if block_size == 0 {
        return Err("filesystem block size must be non-zero".to_owned());
    }

    let available_bytes = available_blocks
        .checked_mul(block_size)
        .ok_or_else(|| "available filesystem byte count overflowed u128".to_owned())?;
    u64::try_from(available_bytes / BYTES_PER_MIB)
        .map_err(|_| "available filesystem MiB does not fit in u64".to_owned())
}

fn parse_env_u64(name: &str, default: u64) -> Result<u64, String> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|error| format!("invalid {name}={value:?}: {error}")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn parse_env_f64(name: &str, default: f64) -> Result<f64, String> {
    match env::var(name) {
        Ok(value) => {
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid {name}={value:?}: {error}"))?;
            if !parsed.is_finite() || parsed < 0.0 {
                return Err(format!("{name} must be a finite non-negative number"));
            }
            Ok(parsed)
        }
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn parse_mem_available_mb(input: &str) -> Result<u64, String> {
    for line in input.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("MemAvailable:") {
            continue;
        }
        let kib = fields
            .next()
            .ok_or_else(|| "MemAvailable is missing its numeric value".to_owned())?
            .parse::<u64>()
            .map_err(|error| format!("invalid MemAvailable value: {error}"))?;
        if let Some(unit) = fields.next()
            && unit != "kB"
        {
            return Err(format!("unsupported MemAvailable unit: {unit}"));
        }
        return Ok(kib / 1_024);
    }
    Err("/proc/meminfo does not contain MemAvailable".to_owned())
}

fn parse_load_one(input: &str) -> Result<f64, String> {
    let value = input
        .split_whitespace()
        .next()
        .ok_or_else(|| "/proc/loadavg is empty".to_owned())?
        .parse::<f64>()
        .map_err(|error| format!("invalid 1m load average: {error}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err("1m load average must be finite and non-negative".to_owned());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(memory_mb: u64, disk_mb: u64, load_one: f64, cpu_count: u32) -> HostResources {
        HostResources {
            available_memory_mb: memory_mb,
            free_disk_mb: disk_mb,
            load_one,
            cpu_count,
        }
    }

    fn policy() -> ResourcePolicy {
        ResourcePolicy {
            min_available_memory_mb: 4_096,
            min_free_disk_mb: 8_192,
            max_load_per_cpu: 2.0,
        }
    }

    #[test]
    fn parses_mem_available_from_proc_snapshot() {
        let input = "MemTotal:       16384000 kB\nMemAvailable:    6291456 kB\n";
        assert_eq!(parse_mem_available_mb(input).unwrap(), 6_144);
    }

    #[test]
    fn parses_one_minute_load() {
        assert!(
            (parse_load_one("7.25 6.00 5.00 1/100 123\n").unwrap() - 7.25).abs() < f64::EPSILON
        );
    }

    #[test]
    fn parses_statfs_available_space_without_precision_loss() {
        assert_eq!(parse_statfs_available_mb("4194304 4096\n").unwrap(), 16_384);
    }

    #[test]
    fn rejects_malformed_statfs_output() {
        assert!(parse_statfs_available_mb("4194304\n").is_err());
        assert!(parse_statfs_available_mb("4194304 0\n").is_err());
        assert!(parse_statfs_available_mb("x 4096\n").is_err());
    }

    #[test]
    fn healthy_snapshot_is_admitted() {
        let snapshot = snapshot(32_768, 64_000, 14.0, 14);
        assert_eq!(policy().evaluate(snapshot), Admission::Admitted(snapshot));
    }

    #[test]
    fn low_memory_is_classified_without_failure() {
        let decision = policy().evaluate(snapshot(2_048, 64_000, 1.0, 14));
        assert!(matches!(
            decision,
            Admission::Deferred {
                pressure: PressureKind::Memory,
                ..
            }
        ));
    }

    #[test]
    fn low_disk_is_classified_without_failure() {
        let decision = policy().evaluate(snapshot(32_768, 4_096, 1.0, 14));
        assert!(matches!(
            decision,
            Admission::Deferred {
                pressure: PressureKind::Disk,
                ..
            }
        ));
    }

    #[test]
    fn high_load_is_classified() {
        let mut policy = policy();
        policy.max_load_per_cpu = 1.5;
        let decision = policy.evaluate(snapshot(32_768, 64_000, 24.0, 8));
        assert!(matches!(
            decision,
            Admission::Deferred {
                pressure: PressureKind::Load,
                ..
            }
        ));
    }

    #[test]
    fn zero_thresholds_disable_all_gates() {
        let policy = ResourcePolicy {
            min_available_memory_mb: 0,
            min_free_disk_mb: 0,
            max_load_per_cpu: 0.0,
        };
        assert!(matches!(
            policy.evaluate(snapshot(1, 1, 1_000.0, 1)),
            Admission::Admitted(_)
        ));
    }
}
