# Phase 0: Research — On-Demand Infrastructure Verification

> Produced by `/speckit-plan`. Resolves every technical unknown needed for the
> design phase. Each item carries a Decision / Rationale / Alternatives
> considered.

## R1 — kube-rs client construction from a kubeconfig path

**Decision**: use `Kubeconfig::read_from(path)` → `Config::from_custom_kubeconfig(
kubeconfig, KubeConfigOptions::default())` → `Client::try_from(config)`. When no
explicit `--kubeconfig` flag is provided, fall back to `Config::infer()`, which
respects `KUBECONFIG` env var then `~/.kube/config` then in-cluster config.

**Rationale**: `Config::from_custom_kubeconfig` is the documented kube-rs API for
loading a kubeconfig from an explicit path (confirmed in
`kube_client::Config` docs). `Config::infer` handles the env-var and default
paths. Together they implement the spec's precedence (FR-001: flag > env >
default). The crate version is already 4.2.0 in `Cargo.toml` — no version
change.

**Alternatives considered**:
- Shell out to `kubectl proxy` and talk to localhost: rejected — violates the
  no-external-dependencies constraint (Principle V: minimal surface).
- Raw `reqwest` with manual kubeconfig YAML parsing: rejected — reimplements
  kube-rs config loading for no benefit.

## R2 — Multi-document YAML manifest parsing and application

**Decision**: embed the `deploy/*.yaml` manifests at compile time via
`include_str!`. Parse them at runtime into multi-document YAML using `serde_yaml`
0.9's deserializer, each document deserialised into a `serde_json::Value` (a
generic JSON object). Apply each object via kube-rs using the `Dynamic` +
`ApiResource` pattern: extract `apiVersion`/`kind` from the parsed object, derive
`ApiResource` via `kube::discovery::oneshot` or `ApiResource::from_gvk`, then use
`Api::<Dynamic>::all_with()` or `::namespaced_with()` with `.create()` or
`.patch()` (strategic merge patch, `PatchParams::default()`).

For objects that need patching after creation (the ValidatingWebhookConfiguration
needs `caBundle` injected; the Deployment needs the image tag updated from
`capacity-admission-webhook:latest` to the local build), apply a
`Patch::Merge(...)` after the initial create.

**Rationale**: kube-rs does not ship a `kubectl apply` equivalent — the
`Dynamic`+`ApiResource` pattern is the idiomatic way to apply arbitrary manifests
without compiling in concrete types for each. `serde_yaml` is needed only for
this multi-doc parse; it is not already a dependency. Embedding manifests via
`include_str!` keeps the single-binary property (no filesystem reads at runtime).

**Alternatives considered**:
- Shell out to `kubectl apply -f`: rejected (no-external-deps constraint).
- Compile in concrete `kube-rs` typed `Api` calls for each manifest: rejected —
  brittle (any manifest change requires code change), and `ValidatingWebhook-
  Configuration` and `CustomResourceDefinition` types would need feature flags.
- Use `kube-rs` server-side apply (`Patch::Apply`): considered but rejected for
  v1 — strategic merge patch (`Patch::Merge`) is simpler and the tool owns all
  the objects it applies (no concurrent managers).

## R3 — Self-signed TLS certificate generation in-process

**Decision**: use `rcgen` 0.13 (pure-Rust, no OpenSSL dependency) with
`CertificateParams` configured with `SanType::DnsName` entries matching the
in-cluster Service DNS (the same SANs the CI workflow's `csr.conf` uses:
`capacity-admission-webhook`, `capacity-admission-webhook.capacity-admission`,
`capacity-admission-webhook.capacity-admission.svc`). Generate a `KeyPair`, then
`CertificateParams::self_signed(key_pair)`, then extract `.cert.pem()` and
`.key.serialize_pem()` to create the Kubernetes TLS `Secret`.

**Rationale**: the existing CI uses `openssl req -x509` to generate a self-signed
cert with those exact SANs. `rcgen` replaces this with a pure-Rust in-process
generation, keeping the no-external-dependencies property (the tool must not
shell out to `openssl`). The generated cert's CN and SANs match the
`ValidatingWebhookConfiguration`'s `clientConfig.service` reference, so the API
server trusts the webhook's HTTPS endpoint. The same PEM cert is base64-encoded
into the webhook config's `caBundle` (self-signed → the cert IS the CA).

**Alternatives considered**:
- Assume cert-manager is installed and use the `cert-setup.yaml` path: rejected —
  FR-003 explicitly requires not assuming cert-manager.
- Shell out to `openssl req -x509`: rejected (no-external-deps constraint).
- Use `ring` (already a transitive dependency) for key generation + a TLS lib
  for cert assembly: rejected — `rcgen` is purpose-built for X.509 generation
  and much simpler.

## R4 — TLS Secret creation

**Decision**: construct a `k8s_openapi::api::core::v1::Secret` with
`type: kubernetes.io/tls`, `data.tls.crt` and `data.tls.key` set to the base64-
encoded PEM strings from R3, and create it via `Api::<Secret>::namespaced(
client, "capacity-admission").create(...)`. The Deployment mounts this Secret
at `/tls` (per `deployment.yaml`).

**Rationale**: this is the exact manual Secret path documented in the README and
used by the CI workflow — just done programmatically via kube-rs instead of
`kubectl create secret tls`. The `type: kubernetes.io/tls` Secret format is the
standard Kubernetes TLS Secret (PEM key + cert in `data`).

**Alternatives considered**: none — this is the only correct path given the
existing Deployment manifest mounts the Secret at `/tls`.

## R5 — Readiness waiting strategy

**Decision**: poll `Api::<Pod>::namespaced(client, "capacity-admission").list(
...)` with a `labelSelector: app=capacity-admission-webhook` until all pods'
`.status.phase == Running` and `.status.containerStatuses[].ready == true`, with
a configurable timeout (default 120s, matching CI's `kubectl wait --timeout=120s`).
Then poll the `Allocation` CRD singleton's `.status.ceilingCpuMilli` until
non-zero (the CI's same readiness gate: capacity state must be populated before
scenarios run — a zero ceiling is the fail-closed state).

**Rationale**: this mirrors the CI workflow's two-stage readiness gate exactly
(pods Ready → allocation ceiling non-zero), so the tool's readiness semantics
match the proven CI path. Polling via kube-rs `Api::<Pod>::list` replaces
`kubectl wait`.

**Alternatives considered**: use a `watcher`/reflector stream instead of polling:
  overkill for a CLI tool that waits once; simple polling with `tokio::time::sleep`
  between attempts is adequate and simpler.

## R6 — Scenario: submit a pod and observe admit/deny

**Decision**: construct a `k8s_openapi::api::core::v1::Pod` with explicit
`spec.containers[].resources.requests` (cpu/memory), create it via
`Api::<Pod>::namespaced(client, "default").create(...)`, and inspect the result:
- On success (admit): the Pod object is returned; assert it was created.
- On error (deny): the `kube::Error::Api(e)` carries `e.code == 403` (the
  webhook's rejection HTTP status) and `e.message` contains the budget-exceeded
  rejection message.

**Rationale**: the webhook's rejection surfaces as an HTTP 403 from the API
server (per `src/webhook/error.rs`). `kube::Error::Api(ErrorResponse)` is the
kube-rs error variant for API-level rejections. Parsing `e.code` and `e.message`
lets the scenario assert both that the pod was denied and that the rejection
message is the expected budget-exceeded format.

**Note**: `kubectl run --requests` was removed in newer kubectl versions (the CI
already works around this by applying explicit Pod manifests); the tool avoids
this entirely by constructing the Pod object in code.

## R7 — Scenario: runtime budget adjustment

**Decision**: patch the Allocation singleton's `spec.budgetPercent` via
`Api::<Allocation>::all(client).patch("cluster-allocation", &PatchParams::
default(), &Patch::Merge(json!({"spec":{"budgetPercent": X}})))`. Then submit a
test pod and verify the admission decision reflects the new ceiling. No restart.

**Rationale**: this is exactly the runtime budget adjustment path documented in
the README (`kubectl patch allocation ...`) and exercised by the webhook's
existing integration tests. Using `Patch::Merge` (not `Patch::Apply` — see the
kube-rs Patch::Merge status envelope gotcha in memory) with the `"spec"` key
wrapped correctly. The webhook's in-process cache picks up the change on the
next decision.

## R8 — Scenario: dry-run mode

**Decision**: patch `spec.enforcementMode` to `"dry-run"` via the same
`Patch::Merge` path as R7. Submit an over-budget pod. Assert it is admitted
(Pod created successfully) — the warning is not directly observable via the
kube-rs create call (warnings surface in `kubectl` output, not in the API
response object). To verify the warning was emitted, check the webhook's
metrics endpoint (`/metrics`) for a `verdict="dry_run_deny"` counter increment,
or check cluster events for the warning.

**Rationale**: the dry-run warning is surfaced via the admission response
`warnings` field, which `kubectl` surfaces but the raw API create response does
not include as a first-class field. The metrics counter
(`capacity_admission_verdicts_total{verdict="dry_run_deny"}`) is the reliable
machine-readable signal that the dry-run path was exercised. The tool scrapes
metrics via the Service's plaintext HTTP port (port-forward is not needed — the
tool runs outside the cluster and can reach the Service via the API proxy:
`/api/v1/namespaces/capacity-admission/services/capacity-admission-webhook:metrics/
/proxy/metrics`).

**Alternatives considered**: port-forward the metrics port: rejected —
  port-forward requires a running `kubectl` (no-external-deps). The API proxy
  path works directly via the kube-rs client.

## R9 — Scenario: capacity tracking accuracy

**Decision**: read the `ClusterCapacity` CRD singleton's `status` (via
`Api::<ClusterCapacity>::all(client).get("cluster-capacity")`), then independently
list all nodes via `Api::<Node>::all(client).list(...)` and sum each node's
`.status.allocatable["cpu"]` and `["memory"]` (parsed as Kubernetes resource
quantities using the existing `src/resources/quantity.rs` parser). Assert the
CRD status values match the independently-computed sums.

**Rationale**: this cross-checks the controller's computation against the raw
node data, verifying the supply-side accounting is correct on real
infrastructure. The existing `quantity.rs` parser is imported from the library
crate, reusing tested code.

## R10 — Scenario: metrics and health endpoints

**Decision**: reach the webhook's `/metrics` and `/healthz` endpoints via the
Kubernetes API proxy: `GET /api/v1/namespaces/capacity-admission/services/
capacity-admission-webhook:metrics/proxy/metrics` and `/healthz`. These are
plain HTTP GETs via the kube-rs client's underlying `reqwest` (the client has
the API server's base URL and auth). Assert `/healthz` returns `ok` (200) and
`/metrics` returns valid Prometheus exposition format (contains
`capacity_admission_verdicts_total`).

**Rationale**: the API proxy (`.../services/<svc>:<port>/proxy/<path>`) lets the
external tool reach the in-cluster Service without port-forwarding. This is the
same mechanism `kubectl proxy` uses, but invoked directly via the kube-rs
client's HTTP layer.

## R11 — Scenario: active degradation (fail-closed simulation)

**Decision**: three degradation scenarios, each followed by restoration:
1. **Kill webhook pods**: `Api::<Pod>::namespaced(client, "capacity-admission")
   .delete_collection(...)` with the `app=capacity-admission-webhook` label
   selector. The Deployment controller recreates them, but during the window
   they are gone, a pod submission must be rejected by the API server
   (`failurePolicy: Fail`). After asserting rejection, wait for the Deployment
   to recreate pods (poll for Ready again).
2. **Delete CRD instances**: delete the `cluster-capacity` and `cluster-
   allocation` singleton instances. The controllers will auto-recreate them
   (spec-003 singleton autocreation), but during the window, a pod submission
   must be rejected (`capacity_data_missing`). After asserting rejection, wait
   for the controllers to recreate + repopulate the singletons.
3. **Induce stale data**: the controllers write `lastUpdated` timestamps. To
   induce staleness, patch the Allocation `status.lastUpdated` to a timestamp
   older than the freshness timeout (`--capacity-freshness-timeout-secs`, default
   30s). Then submit a pod — the webhook must reject (`capacity_data_stale`).
   After asserting rejection, the controllers will naturally re-write a fresh
   `lastUpdated` on the next reconcile.

**Rationale**: each scenario exercises a distinct fail-closed path from the
webhook's error matrix, on real infrastructure. The restoration step ensures
the next scenario starts from a known-good baseline. The `delete_collection`
API call is the kube-rs equivalent of `kubectl delete pods -l ...`. The
controllers' auto-recreation behavior (spec-003) makes degradation reversible
within the throwaway cluster.

**Alternatives considered**: inducing staleness by pausing the controller (e.g.
   killing the controller task) is not feasible externally — the controller runs
   inside the webhook binary. Patching the status timestamp is the direct path.

## R12 — Teardown ordering

**Decision**: delete in reverse dependency order:
1. ValidatingWebhookConfiguration (stop the API server forwarding to the webhook)
2. Deployment (stop the webhook pods)
3. Service
4. TLS Secret
5. RBAC (ClusterRoleBinding, ClusterRole, ServiceAccount)
6. CRD Instances (cluster-allocation, cluster-capacity) — must be deleted
   BEFORE the CRDs themselves
7. CRDs (CustomResourceDefinition: allocations, clustercapacities)
8. Namespace (capacity-admission) — deletes any remaining namespaced resources

Each deletion waits for the object to be fully removed (poll `.get()` until 404)
before proceeding to the next, because Kubernetes finalizers (especially on CRDs
and the namespace) make deletion asynchronous.

**Rationale**: reverse dependency order prevents the API server from erroring on
cascade deletes (e.g. deleting a CRD before its instances leaves orphaned
instances; deleting a namespace before its contents may leave finalizers stuck).
Waiting for each deletion to complete prevents race conditions.

**Alternatives considered**: delete the namespace first and let cascade handle
  the rest: rejected — namespace deletion is slow (finalizers) and CRDs are
  cluster-scoped (not deleted by namespace cascade).

## R13 — Report module design (pure, unit-testable)

**Decision**: the report module (`src/bin/erw-verify/report.rs`) is pure — it
takes a `Vec<ScenarioResult>` (name, status: Pass/Fail/Skip, duration,
optional failure_detail) and renders either human-readable text or JSON. It does
no I/O. This makes it fully unit-testable (Principle VIII): unit tests construct
`ScenarioResult` vectors and assert the rendered output.

**Rationale**: keeping I/O out of the report module means the rendering logic
(formatting, color codes, JSON structure, exit-code derivation) is testable
without a cluster. The scenario runner produces `ScenarioResult` values; the
report module consumes them. Clean separation.

**Exit-code semantics**: 0 = all passed; 1 = one or more scenarios failed; 2 =
setup error (cluster unreachable, manifests failed to apply); 3 = teardown
partial failure (scenarios may have passed but the cluster is not clean).

## R14 — CLI arg parsing approach

**Decision**: hand-rolled arg parsing matching `src/config.rs`'s existing style
(scan `argv` for `--flag value` pairs), NOT a crate like `clap`. This is
consistent with the project's Constitution Principle V (minimal surface) — the
webhook binary already parses its 7 flags this way without a parsing crate.

**Rationale**: the verify tool has a small, fixed flag surface
(`--kubeconfig`, `--json`, `--keep-on-failure`, `--timeout-secs`). Adding `clap`
for 4 flags would pull in a large dependency for marginal benefit and break
consistency with the existing binary's approach.

**Alternatives considered**: `clap` (derive): rejected — large dependency for a
  small fixed surface, inconsistent with `config.rs`.

## R15 — Test strategy for the verify tool itself

**Decision**: three tiers:
1. **Unit tests** (`tests/verify/report.rs`, `tests/verify/args.rs`): pure
   modules — report rendering (human + JSON), exit-code derivation, CLI arg
   parsing edge cases (missing kubeconfig, invalid flag values). These run in
   `cargo test` with no cluster.
2. **No mocked-apiserver integration tests**: the verify tool's core logic
   (apply manifests, wait for readiness, run scenarios, teardown) is inherently
   cluster-dependent. Mocking the apiserver for it would test the mock, not the
   tool. The existing tower-test mocked-apiserver harness is for the webhook's
   admission logic, not for cluster lifecycle operations.
3. **Manual / CI integration**: the tool is exercised end-to-end against a real
   `kind` cluster, either manually by the operator or (optionally) as a CI job
   that runs `erw-verify` against the same `kind` cluster the E2E job creates.
   This is a CI workflow concern, not a production-code concern — the tool is
   the test.

**Rationale**: the verify tool is itself an integration test harness. Its
unit-testable surface (report, args) gets unit tests (Principle VIII); its
cluster-interacting surface is validated by running it against real
infrastructure, which is its entire purpose.

## R16 — Cluster-cleanness safety heuristic (FR-019)

**Decision**: before setup, the tool lists pods in the `default` namespace. If
any non-system pod exists (excluding well-known system pods like
`kube-apiserver`, CNI pods, etc. — which wouldn't be in `default` anyway), the
tool refuses to proceed with an error: "Target cluster is not empty (found N
pods in default namespace). This tool actively degrades the webhook
installation and must only be run against a clean, throwaway cluster."

The heuristic is intentionally simple: check `default` namespace only. It is a
safety net, not a guarantee — the operator's throwaway-cluster commitment (per
the spec) is the real safety boundary.

**Rationale**: the tool actively kills pods and deletes CRDs (R11). Running it
against a cluster with real workloads could disrupt them. The `default`-
namespace check catches the most common mistake (pointing at a dev cluster with
running workloads) without being so strict it rejects legitimate empty clusters.

**Alternatives considered**: check ALL namespaces: rejected — system namespaces
  (`kube-system`) always have pods; distinguishing system from workload pods is
  fragile. A `--force` / `--i-know-what-im-doing` override flag: deferred to a
  future iteration if the heuristic proves too strict in practice.

## R17 — Rustls CryptoProvider (known gotcha)

**Decision**: the verify binary must call
`rustls::crypto::ring::default_provider().install_default().expect("...")` as
the **first line of `main()`**, before constructing the kube-rs client. This is
the same gotcha documented in memory for the webhook binary
(axum-server `tls-rustls-no-provider` + kube-rs `Client::try_default()` opens a
TLS connection that panics without an installed provider).

**Rationale**: the verify binary uses `kube-rs` with `rustls-tls` feature,
which hits the same provider-auto-detection gap. Install the ring provider
before any TLS operation.

## R18 — Dependency version summary

| Dependency | Version | Status | Purpose |
|-----------|---------|--------|----------|
| kube | 4.2.0 | existing (unchanged) | client, config, dynamic apply |
| k8s-openapi | 0.28.0 | existing (unchanged) | Pod, Node, Secret, Deployment types |
| tokio | 1 (full) | existing (unchanged) | async runtime |
| serde / serde_json | 1 | existing (unchanged) | manifest + report JSON |
| tracing / tracing-subscriber | 0.1 / 0.3 | existing (unchanged) | structured logging |
| thiserror | 2 | existing (unchanged) | error enums |
| **rcgen** | **0.13** | **NEW** | self-signed TLS cert generation |
| **serde_yaml** | **0.9** | **NEW** | multi-doc manifest YAML parsing |
| **base64** | **0.22** | **NEW** | base64-encode cert into Secret/TLS config |

All new dependencies are pure-Rust, widely used, and compatible with the
existing toolchain (MSRV 1.89). `rcgen` 0.13 requires the `crypto` feature
implicit via `KeyPair::generate`.
