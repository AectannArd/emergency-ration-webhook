# Quickstart: S10 Degradation Restore Fix

## Prerequisites

- A clean, throwaway Kubernetes cluster
- A kubeconfig for that cluster
- The webhook image already pushed to a registry accessible from the cluster

## Run

```bash
cargo build --bin erw-verify
./target/debug/erw-verify --kubeconfig <path-to-kubeconfig> --skip-build
```

(Or use the spec-009 `.env`-driven pipeline once implemented.)

## Expected outcome

All 11 scenarios pass, including S10:
```
✓ S10  CRD instances deleted → admission rejected [Ns]
  admission rejected: capacity data unavailable: ...
```

## Verification

The fix is confirmed when S10 reports a "capacity data unavailable" rejection
(not an "unreachable" / "no endpoints" rejection).
