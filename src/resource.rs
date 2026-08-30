use std::env;
use std::fs;

const DEFAULT_MIN_AVAILABLE_MEMORY_MB: u64 = 4_096;
const DEFAULT_MAX_LOAD_PER_CPU: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourcePolicy {
    pub min_available_memory_mb: u64,
    pub max_load_per_cpu: f64,
}

impl ResourcePolicy {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            min_available_memory_mb: parse_env_u64(
                "ORCHESTRATOR_MIN_AVAILABLE_MEMORY_MB",
                DEFAULT_MIN_AVAILABLE_MEMORY_MB,
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
                reason: format!(
                    "available memory {} MiB is below required {} MiB",
                    snapshot.available_memory_mb, self.min_available_memory_mb
                ),
            };
        }

        let normalized_load = snapshot.load_per_cpu();
        if self.max_load_per_cpu > 0.0 && normalized_load > self.max_load_per_cpu {
            return Admission::Deferred {
                snapshot,
                reason: format!(
                    "1m load per CPU {:.2} exceeds configured {:.2}",
                    normalized_load, self.max_load_per_cpu
                ),
            };
        }

        Admission::Admitted(snapshot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostResources {
    pub available_memory_mb: u64,
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
        reason: String,
    },
}

pub fn sample_linux() -> Result<HostResources, String> {
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
        load_one: parse_load_one(&loadavg)?,
        cpu_count,
    })
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

    fn snapshot(memory_mb: u64, load_one: f64, cpu_count: u32) -> HostResources {
        HostResources {
            available_memory_mb: memory_mb,
            load_one,
            cpu_count,
        }
    }

    #[test]
    fn parses_mem_available_from_proc_snapshot() {
        let input = "MemTotal:       16384000 kB\nMemAvailable:    6291456 kB\n";
        assert_eq!(parse_mem_available_mb(input).unwrap(), 6_144);
    }

    #[test]
    fn parses_one_minute_load() {
        assert!((parse_load_one("7.25 6.00 5.00 1/100 123\n").unwrap() - 7.25).abs() < f64::EPSILON);
    }

    #[test]
    fn healthy_snapshot_is_admitted() {
        let policy = ResourcePolicy {
            min_available_memory_mb: 4_096,
            max_load_per_cpu: 2.0,
        };
        assert_eq!(
            policy.evaluate(snapshot(32_768, 14.0, 14)),
            Admission::Admitted(snapshot(32_768, 14.0, 14))
        );
    }

    #[test]
    fn low_memory_defers_without_failure() {
        let policy = ResourcePolicy {
            min_available_memory_mb: 4_096,
            max_load_per_cpu: 2.0,
        };
        let decision = policy.evaluate(snapshot(2_048, 1.0, 14));
        assert!(matches!(decision, Admission::Deferred { .. }));
    }

    #[test]
    fn high_normalized_load_defers() {
        let policy = ResourcePolicy {
            min_available_memory_mb: 4_096,
            max_load_per_cpu: 1.5,
        };
        let decision = policy.evaluate(snapshot(32_768, 24.0, 8));
        assert!(matches!(decision, Admission::Deferred { .. }));
    }

    #[test]
    fn zero_thresholds_disable_both_gates() {
        let policy = ResourcePolicy {
            min_available_memory_mb: 0,
            max_load_per_cpu: 0.0,
        };
        assert!(matches!(
            policy.evaluate(snapshot(1, 1_000.0, 1)),
            Admission::Admitted(_)
        ));
    }
}
