//! Kubernetes `resource.Quantity` parsing into integer internal representation.
//!
//! - CPU is normalised to **milli-CPUs** (`i64`): `"500m"` → 500, `"2"` → 2000.
//! - Memory is normalised to **bytes** (`i64`): `"1Gi"` → 1073741824,
//!   `"1G"` → 1000000000, `"1073741824"` → 1073741824.
//!
//! See `data-model.md` §5 for the parsing rules.

use k8s_openapi::api::core::v1::{Container, PodSpec};
use thiserror::Error;

/// Error returned when a resource quantity string cannot be parsed.
#[derive(Debug, Error)]
pub enum QuantityParseError {
    #[error("invalid resource quantity {input:?}: {reason}")]
    Invalid { input: String, reason: &'static str },
}

/// Parse a CPU quantity string into milli-CPUs.
///
/// - `m` suffix → value is already in millicores (`"500m"` → 500).
/// - no suffix → value is in cores, multiplied by 1000 (`"2"` → 2000,
///   `"0.5"` → 500).
pub fn parse_cpu(input: &str) -> Result<i64, QuantityParseError> {
    if input.is_empty() {
        return Err(QuantityParseError::Invalid {
            input: input.into(),
            reason: "empty quantity",
        });
    }
    let (num, suffix) = split_suffix(input, &["m"]);
    let milli = match suffix {
        "m" => parse_number(num)?,
        "" => parse_number(num)? * 1000.0,
        _ => unreachable!("split_suffix only yields suffixes from the given list"),
    };
    Ok(to_i64(milli))
}

/// Parse a memory quantity string into bytes.
///
/// - IEC suffixes (`Ki`,`Mi`,`Gi`,`Ti`,`Pi`) → powers of 1024.
/// - SI suffixes (`k`,`M`,`G`,`T`,`P`) → powers of 1000.
/// - no suffix → bare bytes (`"1073741824"`).
pub fn parse_memory(input: &str) -> Result<i64, QuantityParseError> {
    if input.is_empty() {
        return Err(QuantityParseError::Invalid {
            input: input.into(),
            reason: "empty quantity",
        });
    }
    let suffixes = ["Ki", "Mi", "Gi", "Ti", "Pi", "k", "M", "G", "T", "P"];
    let (num, suffix) = split_suffix(input, &suffixes);

    // Bare pure-integer bytes are parsed as i64 directly so values up to
    // i64::MAX stay exact (cluster totals can be large; f64 would lose
    // precision past 2^53).
    let is_pure_integer =
        suffix.is_empty() && !num.contains('.') && !num.contains('e') && !num.contains('E');
    if is_pure_integer {
        let value: i64 = num.parse().map_err(|_| QuantityParseError::Invalid {
            input: input.into(),
            reason: "not a valid integer",
        })?;
        if value < 0 {
            return Err(QuantityParseError::Invalid {
                input: input.into(),
                reason: "negative quantities are not allowed",
            });
        }
        return Ok(value);
    }

    let multiplier = match suffix {
        "" => 1.0,
        "Ki" => 1024.0,
        "Mi" => (1_i64 << 20) as f64,
        "Gi" => (1_i64 << 30) as f64,
        "Ti" => (1_i64 << 40) as f64,
        "Pi" => (1_i64 << 50) as f64,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        _ => {
            return Err(QuantityParseError::Invalid {
                input: input.into(),
                reason: "unknown resource suffix",
            });
        }
    };
    let bytes = parse_number(num)? * multiplier;
    Ok(to_i64(bytes))
}

/// Strip the longest matching suffix from `input`; returns `(number, suffix)`.
fn split_suffix<'a>(input: &'a str, suffixes: &[&'a str]) -> (&'a str, &'a str) {
    for &suffix in suffixes {
        if let Some(stripped) = input.strip_suffix(suffix) {
            return (stripped, suffix);
        }
    }
    (input, "")
}

/// Parse a non-negative decimal number (cores or a suffix multiplier operand).
fn parse_number(num: &str) -> Result<f64, QuantityParseError> {
    if num.is_empty() {
        return Err(QuantityParseError::Invalid {
            input: num.into(),
            reason: "missing numeric value",
        });
    }
    let value: f64 = num.parse().map_err(|_| QuantityParseError::Invalid {
        input: num.into(),
        reason: "not a valid number",
    })?;
    if value < 0.0 {
        return Err(QuantityParseError::Invalid {
            input: num.into(),
            reason: "negative quantities are not allowed",
        });
    }
    Ok(value)
}

/// Convert an f64 quantity to i64, rounding to the nearest integer and
/// saturating on overflow (huge inputs clamp to i64::MAX rather than wrapping).
fn to_i64(value: f64) -> i64 {
    value.round() as i64
}

/// Effective CPU/memory request for a single container, applying the Kubernetes
/// defaulting convention: a resource missing from `requests` falls back to its
/// `limits` value; a resource present in neither contributes 0.
fn container_requests(container: &Container) -> Result<(i64, i64), QuantityParseError> {
    let resources = container.resources.as_ref();
    let requests = resources.and_then(|r| r.requests.as_ref());
    let limits = resources.and_then(|r| r.limits.as_ref());

    let resolve = |resource: &str,
                   parse: fn(&str) -> Result<i64, QuantityParseError>|
     -> Result<i64, QuantityParseError> {
        // Defaulting: a request missing from `requests` falls back to `limits`.
        let quantity = requests
            .and_then(|m| m.get(resource))
            .or_else(|| limits.and_then(|m| m.get(resource)));
        match quantity {
            Some(q) => parse(&q.0),
            None => Ok(0),
        }
    };

    let cpu = resolve("cpu", parse_cpu)?;
    let memory = resolve("memory", parse_memory)?;
    Ok((cpu, memory))
}

/// Sum the resource requests of a pod's containers into effective milli-CPUs and
/// bytes.
///
/// Regular containers run concurrently, so their requests are summed. Init
/// containers run sequentially, so the effective init requirement per resource
/// is the maximum single init container; the pod's effective request per
/// resource is then `max(sum(regular), max(init))`, matching kube-scheduler.
pub fn extract_pod_requests(spec: &PodSpec) -> Result<(i64, i64), QuantityParseError> {
    let mut sum_regular_cpu = 0i64;
    let mut sum_regular_mem = 0i64;
    for container in &spec.containers {
        let (cpu, mem) = container_requests(container)?;
        sum_regular_cpu += cpu;
        sum_regular_mem += mem;
    }

    // Init containers run sequentially: the effective init requirement per
    // resource is the maximum single init container's request.
    let mut max_init_cpu = 0i64;
    let mut max_init_mem = 0i64;
    if let Some(init_containers) = &spec.init_containers {
        for container in init_containers {
            let (cpu, mem) = container_requests(container)?;
            max_init_cpu = max_init_cpu.max(cpu);
            max_init_mem = max_init_mem.max(mem);
        }
    }

    Ok((
        sum_regular_cpu.max(max_init_cpu),
        sum_regular_mem.max(max_init_mem),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- CPU parsing ----

    #[test]
    fn cpu_milli_suffix() {
        assert_eq!(parse_cpu("500m").unwrap(), 500);
        assert_eq!(parse_cpu("100m").unwrap(), 100);
        assert_eq!(parse_cpu("1000m").unwrap(), 1000);
    }

    #[test]
    fn cpu_bare_cores() {
        assert_eq!(parse_cpu("2").unwrap(), 2000);
        assert_eq!(parse_cpu("1").unwrap(), 1000);
        assert_eq!(parse_cpu("0.5").unwrap(), 500);
        assert_eq!(parse_cpu("1.5").unwrap(), 1500);
    }

    #[test]
    fn cpu_zero() {
        assert_eq!(parse_cpu("0").unwrap(), 0);
        assert_eq!(parse_cpu("0m").unwrap(), 0);
    }

    // ---- Memory parsing ----

    #[test]
    fn memory_iec_suffixes() {
        assert_eq!(parse_memory("1Ki").unwrap(), 1024);
        assert_eq!(parse_memory("512Mi").unwrap(), 536_870_912);
        assert_eq!(parse_memory("1Gi").unwrap(), 1_073_741_824);
        assert_eq!(parse_memory("2Gi").unwrap(), 2_147_483_648);
        assert_eq!(parse_memory("1Ti").unwrap(), 1_099_511_627_776);
    }

    #[test]
    fn memory_si_suffixes() {
        assert_eq!(parse_memory("1k").unwrap(), 1000);
        assert_eq!(parse_memory("1M").unwrap(), 1_000_000);
        assert_eq!(parse_memory("1G").unwrap(), 1_000_000_000);
    }

    #[test]
    fn memory_bare_bytes() {
        assert_eq!(parse_memory("1073741824").unwrap(), 1_073_741_824);
        assert_eq!(parse_memory("0").unwrap(), 0);
    }

    #[test]
    fn memory_max_i64_bare_is_exact() {
        // Bare integers must be parsed as i64 (not via f64) to stay exact up to
        // i64::MAX — cluster totals can be large.
        assert_eq!(parse_memory("9223372036854775807").unwrap(), i64::MAX);
    }

    // ---- Invalid inputs ----

    #[test]
    fn cpu_invalid_inputs() {
        assert!(parse_cpu("").is_err());
        assert!(parse_cpu("abc").is_err());
        assert!(parse_cpu("m").is_err());
        assert!(parse_cpu("1.2.3").is_err());
        assert!(parse_cpu("-5").is_err());
        assert!(parse_cpu("1x").is_err());
    }

    #[test]
    fn memory_invalid_inputs() {
        assert!(parse_memory("").is_err());
        assert!(parse_memory("Gi").is_err());
        assert!(parse_memory("abc").is_err());
        assert!(parse_memory("-5Gi").is_err());
        assert!(parse_memory("1x").is_err());
        assert!(parse_memory("5m").is_err()); // 'm' is not a memory suffix
    }

    // ---- Pod request extraction ----

    use k8s_openapi::api::core::v1::ResourceRequirements;
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

    fn container_with_requests(cpu: &str, mem: &str) -> Container {
        let mut requests = BTreeMap::new();
        requests.insert("cpu".to_string(), Quantity(cpu.to_string()));
        requests.insert("memory".to_string(), Quantity(mem.to_string()));
        Container {
            resources: Some(ResourceRequirements {
                requests: Some(requests),
                limits: None,
                claims: None,
            }),
            ..Default::default()
        }
    }

    fn container_with_limits(cpu: &str, mem: &str) -> Container {
        let mut limits = BTreeMap::new();
        limits.insert("cpu".to_string(), Quantity(cpu.to_string()));
        limits.insert("memory".to_string(), Quantity(mem.to_string()));
        Container {
            resources: Some(ResourceRequirements {
                requests: None,
                limits: Some(limits),
                claims: None,
            }),
            ..Default::default()
        }
    }

    fn pod_spec(containers: Vec<Container>, init: Vec<Container>) -> PodSpec {
        PodSpec {
            containers,
            init_containers: if init.is_empty() { None } else { Some(init) },
            ..Default::default()
        }
    }

    #[test]
    fn pod_explicit_requests() {
        let spec = pod_spec(vec![container_with_requests("500m", "1Gi")], vec![]);
        assert_eq!(extract_pod_requests(&spec).unwrap(), (500, 1_073_741_824));
    }

    #[test]
    fn pod_multi_container_sum() {
        let spec = pod_spec(
            vec![
                container_with_requests("1", "1Gi"),
                container_with_requests("2", "1Gi"),
            ],
            vec![],
        );
        // cpu: 1000 + 2000 = 3000; memory: 2 GiB
        assert_eq!(extract_pod_requests(&spec).unwrap(), (3000, 2_147_483_648));
    }

    #[test]
    fn pod_limits_default_to_requests() {
        // FR-005: requests missing but limits present → requests = limits.
        let spec = pod_spec(vec![container_with_limits("1", "1Gi")], vec![]);
        assert_eq!(extract_pod_requests(&spec).unwrap(), (1000, 1_073_741_824));
    }

    #[test]
    fn pod_no_resources_is_zero() {
        let spec = pod_spec(vec![Container::default()], vec![]);
        assert_eq!(extract_pod_requests(&spec).unwrap(), (0, 0));
    }

    #[test]
    fn pod_empty_is_zero() {
        let spec = pod_spec(vec![], vec![]);
        assert_eq!(extract_pod_requests(&spec).unwrap(), (0, 0));
    }

    #[test]
    fn pod_init_dominates_regular() {
        // regular sum cpu = 1000; single init cpu = 2000 → effective 2000.
        let spec = pod_spec(
            vec![container_with_requests("1", "1Gi")],
            vec![container_with_requests("2", "512Mi")],
        );
        assert_eq!(extract_pod_requests(&spec).unwrap(), (2000, 1_073_741_824));
    }

    #[test]
    fn pod_regular_dominates_init() {
        // regular sum cpu = 3000; init cpu = 1000 → effective 3000.
        let spec = pod_spec(
            vec![container_with_requests("3", "1Gi")],
            vec![container_with_requests("1", "512Mi")],
        );
        assert_eq!(extract_pod_requests(&spec).unwrap(), (3000, 1_073_741_824));
    }

    #[test]
    fn pod_max_of_multiple_init() {
        // two init containers cpu 1 and 5 → max init cpu = 5000.
        let spec = pod_spec(
            vec![container_with_requests("2", "1Gi")],
            vec![
                container_with_requests("1", "1Gi"),
                container_with_requests("5", "1Gi"),
            ],
        );
        assert_eq!(extract_pod_requests(&spec).unwrap(), (5000, 1_073_741_824));
    }

    #[test]
    fn pod_unparseable_quantity_errors() {
        let spec = pod_spec(
            vec![container_with_requests("not-a-quantity", "1Gi")],
            vec![],
        );
        assert!(extract_pod_requests(&spec).is_err());
    }
}
