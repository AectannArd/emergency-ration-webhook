//! Webhook stack setup (spec-005, research R2-R5, R16): apply the embedded
//! `deploy/*.yaml` manifests, generate a self-signed TLS certificate in-process,
//! inject it into the webhook-config `caBundle`, wait for readiness, and run the
//! cluster-cleanness pre-flight check.
//!
//! Manifests are applied with **server-side apply** (`Patch::Apply`), not
//! strategic merge patch. Research R2 preferred `Patch::Merge`, but JSON merge
//! patch cannot create-or-update (it 404s on a missing object) and it replaces
//! whole arrays — which would clobber the webhook-config `webhooks[]` when
//! injecting `caBundle`. SSA is the correct `kubectl apply` semantic: one call
//! creates-or-updates and merges field-by-field. The Allocation *spec* patches
//! in the scenarios (S3-S6) still use `Patch::Merge` per research R7 (spec is a
//! map; merge patches maps correctly).

use std::collections::BTreeMap;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use k8s_openapi::api::core::v1::{Namespace, Pod, Secret};
use kube::Client;
use kube::api::{Api, DynamicObject, ListParams, ObjectMeta, Patch, PatchParams};
use kube::core::gvk::GroupVersionKind;
use kube::discovery::{ApiResource, Scope, pinned_kind};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use serde::Deserialize;
use tracing::{info, warn};

use capacity_admission_webhook::crd::{Allocation, CLUSTER_ALLOCATION_NAME};

use crate::error::{Result, err};

/// Namespace the webhook and its CRDs live in.
const NAMESPACE: &str = "capacity-admission";
/// TLS Secret the Deployment mounts at `/tls`.
const TLS_SECRET_NAME: &str = "capacity-admission-webhook-tls";
/// ValidatingWebhookConfiguration name (deploy/webhook-config.yaml).
const WEBHOOK_CONFIG_NAME: &str = "capacity-admission.emergency-ration.dev";
/// The single webhook's name (== the configuration name).
const WEBHOOK_NAME: &str = "capacity-admission.emergency-ration.dev";
/// Label selector identifying the webhook pods.
const APP_LABEL: &str = "app=capacity-admission-webhook";
/// Server-side-apply field manager identity.
const FIELD_MANAGER: &str = "erw-verify";

/// In-cluster Service DNS SANs the self-signed cert must cover (research R3 —
/// matches the CI `csr.conf` so the apiserver trusts the webhook endpoint).
const SERVICE_DNS_NAMES: [&str; 3] = [
    "capacity-admission-webhook",
    "capacity-admission-webhook.capacity-admission",
    "capacity-admission-webhook.capacity-admission.svc",
];

// Embedded manifests (compiled in — single-binary property, no runtime file reads).
const CRDS: &str = include_str!("../../../deploy/crds.yaml");
const RBAC: &str = include_str!("../../../deploy/rbac.yaml");
const DEPLOYMENT: &str = include_str!("../../../deploy/deployment.yaml");
const WEBHOOK_CONFIG: &str = include_str!("../../../deploy/webhook-config.yaml");

// ---------------------------------------------------------------------------
// T014 — pre-flight: refuse to run against a non-empty cluster (research R16).
// ---------------------------------------------------------------------------

/// Verify the target cluster is clean: list pods in the `default` namespace and
/// refuse if any exist. The tool actively degrades the webhook installation
/// (kills pods, deletes CRDs), so it must only run against a throwaway cluster.
pub async fn check_cluster_clean(client: &Client) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
    let list = pods.list(&ListParams::default()).await?;
    if list.items.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = list
        .items
        .iter()
        .map(|p| {
            p.metadata
                .name
                .clone()
                .unwrap_or_else(|| "<unnamed>".into())
        })
        .collect();
    Err(err(format!(
        "cluster is not empty — found {} pod(s) in the default namespace ({}). \
         This tool actively degrades the webhook installation and must only be run \
         against a clean, throwaway cluster.",
        list.items.len(),
        names.join(", ")
    )))
}

// ---------------------------------------------------------------------------
// T010 — apply the embedded manifests (research R2).
// ---------------------------------------------------------------------------

/// Apply all embedded `deploy/*.yaml` manifests in CI dependency order:
/// namespace → RBAC → CRDs → Deployment/Service → webhook-config.
pub async fn apply_manifests(client: &Client) -> Result<()> {
    let deployment_docs = parse_docs(DEPLOYMENT)?;
    let rbac_docs = parse_docs(RBAC)?;
    let crd_docs = parse_docs(CRDS)?;
    let webhook_docs = parse_docs(WEBHOOK_CONFIG)?;

    // The Namespace lives in deployment.yaml; apply it before RBAC/CRDs (the
    // ServiceAccount and webhook pods reference it).
    let namespace_doc = deployment_docs
        .iter()
        .find(|d| kind_is(d, "Namespace"))
        .ok_or_else(|| err("deployment.yaml is missing its Namespace document"))?;
    apply_doc(client, namespace_doc).await?;

    for d in &rbac_docs {
        apply_doc(client, d).await?;
    }
    for d in &crd_docs {
        apply_doc(client, d).await?;
    }
    for d in deployment_docs.iter().filter(|d| !kind_is(d, "Namespace")) {
        apply_doc(client, d).await?;
    }
    for d in &webhook_docs {
        apply_doc(client, d).await?;
    }
    info!(namespace = NAMESPACE, "webhook stack manifests applied");
    Ok(())
}

/// Apply one manifest document via server-side apply (create-or-update).
async fn apply_doc(client: &Client, doc: &serde_json::Value) -> Result<()> {
    let api_version = doc
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err("manifest document missing apiVersion"))?;
    let kind = doc
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err("manifest document missing kind"))?;
    let name = doc
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| err("manifest document missing metadata.name"))?;
    let namespace = doc
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str());

    let gvk = parse_gvk(api_version, kind);
    let (ar, scope) = resolve_api_resource(client, &gvk, namespace).await?;
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    match scope {
        Scope::Namespaced => {
            let ns = namespace.unwrap_or("default");
            let api = Api::<DynamicObject>::namespaced_with(client.clone(), ns, &ar);
            api.patch(name, &pp, &Patch::Apply(doc)).await?;
        }
        Scope::Cluster => {
            let api = Api::<DynamicObject>::all_with(client.clone(), &ar);
            api.patch(name, &pp, &Patch::Apply(doc)).await?;
        }
    }
    tracing::debug!(%api_version, %kind, %name, "applied manifest document");
    Ok(())
}

/// Split `apiVersion` into `(group, version)` and build a [`GroupVersionKind`].
fn parse_gvk(api_version: &str, kind: &str) -> GroupVersionKind {
    let (group, version) = match api_version.rsplit_once('/') {
        Some((g, v)) => (g, v),
        None => ("", api_version),
    };
    GroupVersionKind::gvk(group, version, kind)
}

/// Resolve the [`ApiResource`] + scope for a GVK via discovery, falling back to a
/// guessed plural (all embedded manifests use regular-plural built-in kinds).
async fn resolve_api_resource(
    client: &Client,
    gvk: &GroupVersionKind,
    namespace: Option<&str>,
) -> Result<(ApiResource, Scope)> {
    match pinned_kind(client, gvk).await {
        Ok((ar, caps)) => Ok((ar, caps.scope)),
        Err(e) => {
            warn!(
                group = %gvk.group,
                version = %gvk.version,
                kind = %gvk.kind,
                error = %e,
                "api discovery failed; guessing the resource plural from the kind"
            );
            let scope = if namespace.is_some() {
                Scope::Namespaced
            } else {
                Scope::Cluster
            };
            Ok((ApiResource::from_gvk(gvk), scope))
        }
    }
}

/// Parse a multi-document YAML manifest into JSON values, skipping empty docs.
fn parse_docs(manifest: &str) -> Result<Vec<serde_json::Value>> {
    let mut docs = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(manifest) {
        let value: serde_json::Value = serde_json::Value::deserialize(doc)?;
        if !value.is_null() {
            docs.push(value);
        }
    }
    Ok(docs)
}

fn kind_is(doc: &serde_json::Value, kind: &str) -> bool {
    doc.get("kind").and_then(|v| v.as_str()) == Some(kind)
}

// ---------------------------------------------------------------------------
// T011 — generate the self-signed TLS cert + create the Secret (research R3-R4).
// ---------------------------------------------------------------------------

/// Generate a self-signed TLS certificate in-process (rcgen — no OpenSSL) and
/// create/update the `kubernetes.io/tls` Secret the Deployment mounts at `/tls`.
/// Ensures the namespace exists first. Returns the PEM cert (used to inject the
/// webhook-config `caBundle`).
pub async fn create_tls_secret(client: &Client) -> Result<String> {
    ensure_namespace(client).await?;

    let san_names: Vec<String> = SERVICE_DNS_NAMES.iter().map(|s| (*s).to_string()).collect();
    let mut params = CertificateParams::new(san_names)?;
    // The same cert serves the webhook over HTTPS AND is the caBundle the
    // apiserver trusts (self-signed → the cert is its own CA). Mark it a CA
    // explicitly so its basicConstraints is unambiguous as a trust root.
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let mut data = BTreeMap::new();
    data.insert(
        "tls.crt".to_string(),
        ByteString(STANDARD.encode(cert_pem.as_bytes()).into_bytes()),
    );
    data.insert(
        "tls.key".to_string(),
        ByteString(STANDARD.encode(key_pem.as_bytes()).into_bytes()),
    );
    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(TLS_SECRET_NAME.into()),
            namespace: Some(NAMESPACE.into()),
            ..Default::default()
        },
        data: Some(data),
        type_: Some("kubernetes.io/tls".into()),
        ..Default::default()
    };

    let api: Api<Secret> = Api::namespaced(client.clone(), NAMESPACE);
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    api.patch(TLS_SECRET_NAME, &pp, &Patch::Apply(&secret))
        .await?;
    info!(
        namespace = NAMESPACE,
        secret = TLS_SECRET_NAME,
        "created TLS serving certificate Secret"
    );
    Ok(cert_pem)
}

/// Ensure the webhook namespace exists (idempotent SSA).
async fn ensure_namespace(client: &Client) -> Result<()> {
    let ns = Namespace {
        metadata: ObjectMeta {
            name: Some(NAMESPACE.into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let api: Api<Namespace> = Api::all(client.clone());
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    api.patch(NAMESPACE, &pp, &Patch::Apply(&ns)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// T012 — inject the caBundle into the webhook-config (research R2-R3).
// ---------------------------------------------------------------------------

/// Patch `webhooks[0].clientConfig.caBundle` with the base64-encoded serving
/// cert. Uses server-side apply so only that field is set — JSON merge patch
/// would replace the whole `webhooks[]` array.
pub async fn inject_ca_bundle(client: &Client, cert_pem: &str) -> Result<()> {
    let ca_b64 = STANDARD.encode(cert_pem.as_bytes());
    let patch = serde_json::json!({
        "apiVersion": "admissionregistration.k8s.io/v1",
        "kind": "ValidatingWebhookConfiguration",
        "metadata": { "name": WEBHOOK_CONFIG_NAME },
        "webhooks": [{
            "name": WEBHOOK_NAME,
            "clientConfig": { "caBundle": ca_b64 }
        }]
    });
    let api: Api<k8s_openapi::api::admissionregistration::v1::ValidatingWebhookConfiguration> =
        Api::all(client.clone());
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    api.patch(WEBHOOK_CONFIG_NAME, &pp, &Patch::Apply(&patch))
        .await?;
    info!("injected caBundle into ValidatingWebhookConfiguration");
    Ok(())
}

// ---------------------------------------------------------------------------
// T013 — wait for readiness (research R5): pods Ready, then ceiling non-zero.
// ---------------------------------------------------------------------------

/// Poll until all webhook pods are `Running` + containers ready, then until the
/// Allocation `ceilingCpuMilli` is non-zero (supply known — a zero ceiling is the
/// fail-closed state). Times out (setup error → exit 2) after `timeout`.
pub async fn wait_for_readiness(client: &Client, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let pods: Api<Pod> = Api::namespaced(client.clone(), NAMESPACE);
    let lp = ListParams::default().labels(APP_LABEL);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(err(
                "readiness timeout: webhook pods never reached Ready within the timeout",
            ));
        }
        let list = pods.list(&lp).await?;
        if !list.items.is_empty() && list.items.iter().all(pod_ready) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    info!("webhook pods are Ready");

    let allocs: Api<Allocation> = Api::all(client.clone());
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(err(
                "readiness timeout: Allocation ceiling never became non-zero within the timeout",
            ));
        }
        if let Ok(allocation) = allocs.get(CLUSTER_ALLOCATION_NAME).await
            && let Some(status) = &allocation.status
            && status.ceiling_cpu_milli > 0
        {
            info!(
                ceiling_cpu_milli = status.ceiling_cpu_milli,
                "capacity state populated"
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Whether a pod is fully ready: `Running` phase + every container ready.
fn pod_ready(pod: &Pod) -> bool {
    let Some(status) = pod.status.as_ref() else {
        return false;
    };
    let running = status.phase.as_deref() == Some("Running");
    let containers_ready = status
        .container_statuses
        .as_ref()
        .map(|cs| !cs.is_empty() && cs.iter().all(|c| c.ready))
        .unwrap_or(false);
    running && containers_ready
}

// ByteString lives at the k8s_openapi root.
use k8s_openapi::ByteString;
