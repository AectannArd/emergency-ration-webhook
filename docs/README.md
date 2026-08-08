# Documentation Index

This directory holds the operator-facing reference for
`emergency-ration-webhook`, split into one article per capability. The
[`README`](../README.md) is the entry point — a project overview, a self-contained
quick start, and a one-line summary for each topic below. This index is the local
map of what lives where.

## Getting Started

- [**Deployment Guide**](./deployment.md) — building the image, the 6-step deploy
  sequence, and TLS provisioning (cert-manager or manual Secret).

## Configuration

- [**Configuration Reference**](./configuration.md) — CLI flags and env vars, the
  `Allocation` and `ClusterCapacity` CRD field tables, runtime budget adjustment,
  per-resource overrides, and budget edge cases.
- [**Node Exclusion**](./node-exclusion.md) — the two-layer node filter
  (unschedulable + label selectors), the spec-006→007 migration, and selector
  examples.
- [**Enforcement Modes**](./enforcement-modes.md) — `enforce` vs `dry-run`, the
  fail-closed-in-both-modes contract, and runtime switching.
- [**Workload Exclusion**](./workload-exclusion.md) — namespace and
  priority-class exemption lists, the check order, and the still-counted
  semantics.

## Operations

- [**Metrics & Observability**](./observability.md) — HTTP endpoints, the 8
  Prometheus metrics, structured logging fields, and the rejection message
  format.
- [**Failure Modes**](./failure-modes.md) — every degradation path, the
  fail-closed contract, and the webhook self-admission bootstrap.
- [**Kubernetes Compatibility**](./kubernetes-compatibility.md) — the N-2 support
  window, the CI version matrix, and the GA APIs the webhook depends on.

## Architecture

- [**Architecture**](./architecture.md) — the 3-component operator data flow and
  the two CRDs that link them.

## Tooling

- [**On-Demand Verification (erw-verify)**](./erw-verify.md) — the
  throwaway-cluster verification tool: build, configure, run, CLI flags, exit
  codes, and the S1–S11 / E1–E5 scenario inventory.

## Equalizer

- [**Multi-Cluster Capacity Equalizer**](./equalizer.md) — the separate
  `capacity-equalizer` binary that balances cumulative capacity across a fleet of
  clusters.
