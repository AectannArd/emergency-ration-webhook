//! kube::Client construction from a kubeconfig path (spec-005, research R1).
//!
//! Precedence: explicit `--kubeconfig` path → `Config::infer` (which honours
//! `KUBECONFIG`, then `~/.kube/config`, then in-cluster). The resolved cluster
//! URL is returned alongside the client so the report can name the cluster.
//!
//! The rustls CryptoProvider MUST already be installed before this is called —
//! the orchestrator does that as the first line of `main` (research R17).

use std::path::Path;

use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};

use crate::error::Result;

/// Build a [`Client`] from a kubeconfig path, or via [`Config::infer`] when
/// `None`. Returns `(client, cluster_url)`.
pub async fn build_client(kubeconfig: Option<&Path>) -> Result<(Client, String)> {
    let config = match kubeconfig {
        Some(path) => {
            let kc = Kubeconfig::read_from(path)
                .map_err(|e| format!("reading kubeconfig {path:?}: {e}"))?;
            Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default())
                .await
                .map_err(|e| format!("building kube config from {path:?}: {e}"))?
        }
        None => Config::infer().await.map_err(|e| {
            format!("inferring kubeconfig ({e}); pass --kubeconfig or set KUBECONFIG")
        })?,
    };
    let cluster_url = config.cluster_url.to_string();
    let client = Client::try_from(config)?;
    Ok((client, cluster_url))
}
