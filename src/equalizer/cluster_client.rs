//! Per-target `kube::Client` construction from kubeconfig bytes (spec-013,
//! research R1).
//!
//! The equalizer reads each target cluster's kubeconfig from a `Secret` in the
//! home cluster, parses it, and builds a `kube::Client`. This mirrors the proven
//! `erw-verify/client.rs` pattern — `Kubeconfig::from_yaml` +
//! `Config::from_custom_kubeconfig` — but from in-memory bytes rather than a
//! file path.

use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};

use super::Result;

/// Build a [`kube::Client`] from a target cluster's kubeconfig YAML bytes.
///
/// Mirrors the `erw-verify` client construction (research R1,
/// contracts/target-cluster-api.md §1.2): parse the YAML into a [`Kubeconfig`],
/// resolve a [`Config`] (no network I/O — the connection is lazy), and construct
/// the client. The rustls CryptoProvider MUST already be installed before any
/// TLS-bearing kubeconfig is built (the binary does this as its first line);
/// plain-HTTP kubeconfigs need no provider.
pub async fn build_target_client(kubeconfig_bytes: &[u8]) -> Result<Client> {
    let yaml = std::str::from_utf8(kubeconfig_bytes)
        .map_err(|e| format!("kubeconfig is not valid UTF-8: {e}"))?;
    let kc = Kubeconfig::from_yaml(yaml).map_err(|e| format!("parsing kubeconfig YAML: {e}"))?;
    let config = Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default())
        .await
        .map_err(|e| format!("building kube config from kubeconfig: {e}"))?;
    let client = Client::try_from(config)?;
    Ok(client)
}
