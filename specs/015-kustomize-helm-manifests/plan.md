# Implementation Plan: Kustomize + Helm Manifest Bundles

**Branch**: `spec/015-kustomize-helm-manifests` | **Date**: 2026-08-08 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/015-kustomize-helm-manifests/spec.md`

## Summary

Add two templated manifest distribution formats — Kustomize bundles and Helm
charts — as first-class release artifacts alongside the Docker images. The
Kustomize bundles (`deploy/kustomize/webhook/`, `deploy/kustomize/equalizer/`)
**replace** the existing raw `deploy/*.yaml` and `deploy/equalizer/*.yaml`
files as the single manifest source of truth. The Helm charts
(`deploy/charts/webhook/`, `deploy/charts/equalizer/`) wrap the same resources
with parameterization and ship as versioned `.tgz` packages attached to each
GitHub Release. All existing manifest consumers (`erw-verify`, CI workflows,
documentation) are migrated onto the new bundles, and the raw root manifests are
deleted. The release workflow (`publish.yml`) is extended to package and attach
the charts, `ARTIFACTS.md` is expanded to cover manifest bundles, and the
constitution gains a principle requiring manifest bundles alongside every
containerised artifact.

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024) for the existing binaries
(`capacity-admission-webhook`, `capacity-equalizer`, `erw-verify`); the manifest
bundles themselves are YAML + Go template (Helm) / YAML + kustomization
(Kustomize), not Rust.

**Primary Dependencies** (new, for the manifest bundles only):

- **Kustomize** — the `kustomization.yaml` format. No build-time dependency;
  rendered at apply time by `kubectl -k` / `kustomize build`. The kustomization
  files are static YAML — no Go code, no build step for the repo.
- **Helm 3** — chart scaffolding (`Chart.yaml`, `values.yaml`, `templates/`).
  `helm lint` is the validation gate; `helm package` produces the `.tgz`.
  Go templates are the chart templating engine.
- **`k8s-openapi` + `serde_yaml`** (existing crate, no version change) — used by
  `erw-verify` to parse/render manifests at runtime (replacing the compile-time
  `include_str!` embedding). Already in the dependency tree.
- **GitHub Actions**: `helm/chart-releaser-action` is NOT used (no chart repo —
  release attachments only). The `softprops/action-gh-release` action (or
  equivalent) attaches the `.tgz` files to the release.

**Storage**: N/A — manifest bundles are static files in the repo.

**Testing**:

- **Chart validation**: `helm lint` on each chart (FR-006, FR-014). CI gate.
- **Kustomize parity**: a comparison script/test that renders the Kustomize
  output and asserts field-level equivalence to the pre-migration raw manifests
  (FR-008/FR-009). Implemented as a test script invoked from CI.
- **`erw-verify` scenarios**: S1–S11 (webhook) and E1–E5 (equalizer, CI-only)
  must pass unchanged after migration (FR-016, SC-005). This is the behavioral
  regression gate.
- **CI E2E**: `ci.yml` and `equalizer-e2e.yml` deploy via the new bundles
  (FR-018, FR-019).
- **Integration tests**: unchanged (FR-020) — they use mocked-apiserver fixtures.

**Target Platform**: Kubernetes 1.34–1.36 (N-2 window); manifests must be valid
against the oldest supported version.

**Project Type**: This feature does not add a new Rust binary. It adds
infrastructure artifacts (manifest bundles), reworks an existing tool
(`erw-verify`), reworks CI YAML, and amends governance docs.

**Performance Goals**: N/A — static manifest files, no runtime path.

**Constraints**:

- **Zero behavioral regression**: every consumer of the deleted raw manifests
  (`erw-verify`, CI) must produce identical cluster state after migration. The
  rendered Kustomize output must be field-by-field equivalent to the raw YAML
  (only the image field may differ).
- **No external hosting**: Helm charts are GitHub Release attachments only — no
  OCI registry, no GitHub Pages chart repo (clarification decision).
- **No new container images**: the existing Docker images are unchanged; the
  bundles reference them by the same registry repos.
- **Portable**: no host paths in the bundles; chart `values.yaml` defaults to the
  published Docker Hub images.

**Scale/Scope**: 2 components (webhook + equalizer) × 2 formats (Kustomize +
Helm) = 4 bundle directories. 3 manifest consumers to migrate (`erw-verify`,
`ci.yml`, `equalizer-e2e.yml`). ~9 documentation surfaces to update. 1
constitution amendment.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default | ✅ PASS | The manifest bundles deploy the exact same `failurePolicy: Fail` ValidatingWebhookConfiguration. No change to the admission contract. |
| II | Capacity as a Hard Budget | ✅ PASS | Budget enforcement logic unchanged; the manifests carry the same controller deployment. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The webhook failurePolicy/sideEffects/timeoutSeconds are preserved verbatim in both Kustomize and Helm templates. |
| IV | Observability Before Optimisation | ✅ PASS | The Deployment's ports, probes, metrics annotations, and RUST_LOG env are preserved in the bundles. |
| V | Separated Concerns, Minimal Surface | ✅ PASS | Two independent bundles per format (webhook + equalizer), matching the component split. No new components added. |
| VI | Integration Test Coverage | ✅ PASS | `erw-verify` scenarios + CI E2E are the regression gate. Kustomize parity script adds a new comparison test. |
| VII | Kubernetes N-2 | ✅ PASS | Manifests use the same API versions (already GA across the window). Helm/Kustomize add no new K8s API surface. |
| VIII | Test-First Development | ✅ PASS | The Kustomize parity comparison and `helm lint` are the "tests" for this feature; they are written before the bundles are finalized. The `erw-verify` regression suite is the behavioral gate. |
| IX | Editor Configuration as Code | ✅ PASS | New YAML files (kustomization.yaml, Chart.yaml, values.yaml, templates) must comply with `.editorconfig`. Add Helm template sections if needed. |
| X | README as Documentation Hub | ✅ PASS | README Quick Start updated to Helm/Kustomize (FR-022); new `docs/manifest-bundles.md` article + TOC entry (FR-021, FR-025). |
| XI | CI-Green Completion Gate | ✅ PASS | Both CI E2E workflows pass via the new bundles (FR-018/FR-019); `helm lint` added as a CI gate. |
| XII | Scratch Space | ✅ PASS | Rendered manifests during development go to `.temp/`. |
| XIII | Usage/Contribution Doc Separation | ✅ PASS | `docs/manifest-bundles.md` (usage); `CONTRIBUTING.md` updated for chart packaging + Kustomize deploy (FR-024). |
| XIV | Artifact Inventory | ✅ PASS | `ARTIFACTS.md` expanded to list manifest bundles (FR-015). |
| XV | Build/Publish Procedure for Every Docker Artifact | ⚠️ EXPAND | This principle currently covers Docker images only. It MUST be expanded to require a manifest bundle (Kustomize + Helm) alongside every containerised artifact, with the same same-change obligation. See FR-026 and the new principle (XVI). |

**Gate result**: PASS with one expansion — Principle XV must be broadened (or a
new principle XVI added) to cover manifest bundles as release artifacts. This is
a constitution amendment landing in the same change.

## Project Structure

### Documentation (this feature)

```text
specs/015-kustomize-helm-manifests/
├── plan.md              # This file
├── research.md          # Phase 0: Kustomize/Helm decisions, erw-verify migration approach
├── data-model.md        # Manifest inventory, resource mapping, consumer impact matrix
├── quickstart.md        # Validation scenarios: helm install, kustomize build, parity check
├── contracts/
│   ├── kustomize-bundle.md  # Kustomize layout, overlay contract, image override mechanism
│   ├── helm-chart.md        # Chart structure, values schema, template contract
│   ├── release-workflow.md  # Chart packaging + attachment steps in publish.yml
│   └── erw-verify-migration.md  # How erw-verify consumes Kustomize output
└── tasks.md             # (Phase 2 — created by /speckit-tasks)
```

### Source Code (repository root)

```text
deploy/
├── kustomize/                        # NEW — Kustomize bundles (manifest source of truth)
│   ├── webhook/
│   │   ├── kustomization.yaml        # resources + images directive + namespace
│   │   ├── crds.yaml                 # ClusterCapacity + Allocation CRDs
│   │   ├── deployment.yaml           # Namespace + Deployment + Service
│   │   ├── rbac.yaml                 # ServiceAccount + ClusterRole + ClusterRoleBinding
│   │   ├── webhook-config.yaml       # ValidatingWebhookConfiguration
│   │   └── cert-setup.yaml           # cert-manager Issuer + Certificate
│   └── equalizer/
│       ├── kustomization.yaml
│       ├── crds.yaml                 # EqualizerConfig CRD
│       ├── deployment.yaml           # Namespace + Deployment
│       └── rbac.yaml                 # ServiceAccount + ClusterRole + ClusterRoleBinding
├── charts/                           # NEW — Helm charts (release artifacts)
│   ├── webhook/
│   │   ├── Chart.yaml
│   │   ├── values.yaml
│   │   └── templates/
│   │       ├── namespace.yaml
│   │       ├── crds.yaml
│   │       ├── deployment.yaml
│   │       ├── service.yaml
│   │       ├── rbac.yaml
│   │       ├── webhook-config.yaml
│   │       ├── cert-setup.yaml
│   │       └── _helpers.tpl
│   └── equalizer/
│       ├── Chart.yaml
│       ├── values.yaml
│       └── templates/
│           ├── namespace.yaml
│           ├── crds.yaml
│           ├── deployment.yaml
│           ├── rbac.yaml
│           ├── equalizer-config.example.yaml
│           └── _helpers.tpl
# DELETED: deploy/deployment.yaml, deploy/rbac.yaml, deploy/crds.yaml,
#          deploy/webhook-config.yaml, deploy/cert-setup.yaml
# DELETED: deploy/equalizer/*.yaml

src/bin/erw-verify/
├── setup.rs               # MODIFIED — consume Kustomize-rendered manifests
├── env_config.rs          # MODIFIED (if needed) — image substitution target
└── ...                    # (other modules unchanged)

.github/workflows/
├── ci.yml                 # MODIFIED — kustomize build | kubectl apply
├── equalizer-e2e.yml      # MODIFIED — kustomize build for both stacks
└── publish.yml            # MODIFIED — package + attach Helm charts

ARTIFACTS.md               # MODIFIED — manifest bundle inventory section
CONTRIBUTING.md            # MODIFIED — chart packaging + Kustomize deploy
README.md                  # MODIFIED — Quick Start + TOC
docs/
├── manifest-bundles.md    # NEW — installation via Kustomize + Helm
└── deployment.md          # MODIFIED — paths rewritten to bundles

.specify/memory/constitution.md  # AMENDED — Principle XVI (manifest bundles)
```

**Structure Decision**: Kustomize and Helm bundles live side-by-side under
`deploy/`, each with a per-component subdirectory (`webhook/`, `equalizer/`),
mirroring the existing `deploy/equalizer/` layout. Kustomize is the manifest
source of truth; the Helm charts are an independent parameterized packaging of
the same resources (not generated FROM Kustomize — each chart's templates are
hand-written YAML with Go template directives, validated for parity against the
Kustomize output). The raw root manifests are deleted, not retained as a third
parallel path. `erw-verify` is reworked to render the Kustomize output at
runtime (see `contracts/erw-verify-migration.md`).

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Reworking `erw-verify` from compile-time `include_str!` to runtime Kustomize rendering | The raw manifests it embeds are being deleted; keeping a hidden copy just for `erw-verify` defeats the single-source-of-truth goal | Keeping the raw files as a private `erw-verify`-only copy was rejected by the user ("those at the root should be deleted once you have moved all the processes") |
| Two independent Helm charts instead of one umbrella chart | The webhook and equalizer are independently deployed, in different namespaces, with different CRDs/RBAC — they are separate release artifacts | One umbrella chart (user choice: two independent charts) would couple independently-deployable components and complicate the release `.tgz` naming |

## Constitution Check (Post-Design)

*Re-evaluated after Phase 1 design artifacts.*

| # | Principle | Post-Design Status | Evidence |
|---|-----------|-------------------|----------|
| I–XIV | (as above) | ✅ PASS | Design artifacts (data-model resource mapping, contracts) preserve every field the constitution protects: failurePolicy, sideEffects, RBAC verbs, budget defaults. |
| XV | Build/Publish Procedure | ✅ PASS (after expansion) | The new Principle XVI extends the same-change obligation to manifest bundles; `publish.yml` gains a chart-package-and-attach job (contract `release-workflow.md`). |
| XVI | Manifest Bundle Release Artifacts (NEW) | ✅ PASS | New principle: every containerised artifact ships Kustomize + Helm bundles as release artifacts. |

**Post-design gate**: PASS. No violations introduced by the design.
