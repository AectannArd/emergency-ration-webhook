# Kubernetes Compatibility

[← Back to README](../README.md)

The webhook supports an **N-2 window**: the three most recent Kubernetes releases
(Constitution Principle VII). As of the current implementation, CI tests against
**1.34, 1.35, and 1.36** (source:
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) `e2e` matrix).

All Kubernetes APIs the webhook uses are GA/stable across the window:

- `admissionregistration.k8s.io/v1` — `ValidatingWebhookConfiguration`
- `apiextensions.k8s.io/v1` — `CustomResourceDefinition`
- core `v1` — `Pod`, `Node`

Deprecating support for an older release is a deliberate, documented decision,
not drift.

The webhook's own [self-admission bootstrap](./failure-modes.md#webhook-self-admission-bootstrap)
(the `namespaceSelector` defence-in-depth that keeps the webhook from gating its
own deployment) is documented under [Failure Modes](./failure-modes.md).
