# Tasks: Kustomize + Helm Manifest Bundles

**Input**: Design documents from `/specs/015-kustomize-helm-manifests/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Tests for this feature are validation-driven (`helm lint`, Kustomize↔pre-migration parity, `erw-verify` regression, CI E2E green). They are included as explicit validation tasks per phase, following Constitution Principle VIII (test-first: write the parity check before finalizing bundles).

**Organization**: Tasks are grouped by user story. Spec user stories US1 (Helm), US2 (Kustomize), US3 (release), US4 (migration), US5 (docs) are reordered into **implementation dependency order**: Kustomize bundles (US2) land first because they become the manifest source of truth that all other work depends on.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US5)
- Include exact file paths in descriptions

## Path Conventions

- Kustomize bundles: `deploy/kustomize/<component>/`
- Helm charts: `deploy/charts/<component>/`
- CI workflows: `.github/workflows/`
- Source: `src/bin/erw-verify/`
- Docs: `docs/`, `README.md`, `CONTRIBUTING.md`, `ARTIFACTS.md`

---

## Phase 1: Setup

**Purpose**: Create the directory structure and validation tooling before any bundles are written.

- [ ] T001 Create Kustomize bundle directories: `deploy/kustomize/webhook/` and `deploy/kustomize/equalizer/`
- [ ] T002 Create Helm chart directories with scaffold: `deploy/charts/webhook/{Chart.yaml,values.yaml,templates/_helpers.tpl}` and `deploy/charts/equalizer/{Chart.yaml,values.yaml,templates/_helpers.tpl}`
- [ ] T003 [P] Add `*.tpl` to the YAML section of `.editorconfig` (e.g. `[*.{yaml,yml,tpl}]`) per research R11

---

## Phase 2: Kustomize Bundles — Manifest Source of Truth (US2)

**Goal**: Create the two Kustomize bundles as the single manifest source of truth, producing field-by-field identical resources to the pre-migration raw manifests (data-model §1). This is the foundational phase — all other work depends on these bundles.

**Independent Test**: `kustomize build deploy/kustomize/webhook` and `kustomize build deploy/kustomize/equalizer` render valid YAML; field-level parity with the raw manifests is verified before any consumer migrates.

### Validation — write the parity check first (TDD, Principle VIII)

- [ ] T004 [P] [US2] Create a parity validation script at `.specify/scripts/bash/verify-manifest-parity.sh` that renders `kustomize build deploy/kustomize/webhook` and compares the resource set against the pre-migration raw manifests (`deploy/crds.yaml`, `deploy/rbac.yaml`, `deploy/deployment.yaml`, `deploy/webhook-config.yaml`, `deploy/cert-setup.yaml`), asserting field-level equivalence on all critical fields (data-model §2: failurePolicy, sideEffects, timeoutSeconds, RBAC verbs, names, namespaces, container ports, probes, securityContext). Permitted difference: the image field (placeholder → resolved by kustomization images directive). Exit non-zero on any mismatch.
- [ ] T005 [P] [US2] Extend the parity script (or create `.specify/scripts/bash/verify-equalizer-parity.sh`) to do the same for the equalizer bundle vs `deploy/equalizer/{crds,deployment,rbac}.yaml`.

### Webhook Kustomize bundle

- [ ] T006 [P] [US2] Create `deploy/kustomize/webhook/kustomization.yaml` per contracts/kustomize-bundle.md §2.1 (apiVersion, kind, resources list of all 5 YAML files, images directive: name `capacity-admission-webhook`, newName `aectann/emergency-ration-webhook`, newTag `latest`)
- [ ] T007 [P] [US2] Create `deploy/kustomize/webhook/crds.yaml` — copy `deploy/crds.yaml` content verbatim (ClusterCapacity + Allocation CRDs, data-model W4–W5)
- [ ] T008 [P] [US2] Create `deploy/kustomize/webhook/deployment.yaml` — copy `deploy/deployment.yaml` content (Namespace + Deployment + Service, W1–W3), changing only the container `image:` field from `ERW_IMAGE_PLACEHOLDER` to `capacity-admission-webhook:placeholder` (the kustomization images directive resolves it to the real reference)
- [ ] T009 [P] [US2] Create `deploy/kustomize/webhook/rbac.yaml` — copy `deploy/rbac.yaml` content verbatim (ServiceAccount + ClusterRole + ClusterRoleBinding, W6–W8). Preserve the exact RBAC verb lists (data-model §2.2)
- [ ] T010 [P] [US2] Create `deploy/kustomize/webhook/webhook-config.yaml` — copy `deploy/webhook-config.yaml` content verbatim (ValidatingWebhookConfiguration, W9). Preserve failurePolicy: Fail, sideEffects: None, timeoutSeconds: 5, matchPolicy: Exact, namespaceSelector
- [ ] T011 [P] [US2] Create `deploy/kustomize/webhook/cert-setup.yaml` — copy `deploy/cert-setup.yaml` content verbatim (cert-manager Issuer + Certificate, W10–W11)

### Equalizer Kustomize bundle

- [ ] T012 [P] [US2] Create `deploy/kustomize/equalizer/kustomization.yaml` per contracts/kustomize-bundle.md §2.2 (resources: crds, deployment, rbac; images directive: name `capacity-equalizer`, newName `aectann/emergency-ration-equalizer`, newTag `latest`)
- [ ] T013 [P] [US2] Create `deploy/kustomize/equalizer/crds.yaml` — copy `deploy/equalizer/crds.yaml` content verbatim (EqualizerConfig CRD, E3)
- [ ] T014 [P] [US2] Create `deploy/kustomize/equalizer/deployment.yaml` — copy `deploy/equalizer/deployment.yaml` content (Namespace + Deployment, E1–E2), changing only the image field from `ERW_EQUALIZER_IMAGE_PLACEHOLDER` to `capacity-equalizer:placeholder`
- [ ] T015 [P] [US2] Create `deploy/kustomize/equalizer/rbac.yaml` — copy `deploy/equalizer/rbac.yaml` content verbatim (ServiceAccount + ClusterRole + ClusterRoleBinding, E4–E6), including the target-cluster RBAC comment block (research R10)
- [ ] T016 [P] [US2] Create `deploy/kustomize/equalizer/example-config.yaml` — copy `deploy/equalizer/equalizer-config.example.yaml` content (example kubeconfig Secrets + EqualizerConfig). This file is NOT listed in `kustomization.yaml` `resources:` (it is documentation, not a default-applied resource — research R10)

### Validate

- [ ] T017 [US2] Run `kustomize build deploy/kustomize/webhook` and verify it produces valid multi-document YAML with all 11 webhook resources (data-model §1.1). Run `kustomize build deploy/kustomize/equalizer` and verify all 6 equalizer resources (data-model §1.2)
- [ ] T018 [US2] Run the parity scripts (T004 + T005) and verify zero mismatches against the pre-migration raw manifests. Fix any field drift before proceeding

**Checkpoint**: Kustomize bundles are the manifest source of truth, parity-verified. Consumer migration (Phase 4) and Helm chart authoring (Phase 3) can now proceed.

---

## Phase 3: Helm Charts (US1)

**Goal**: Create two independent Helm charts whose rendered output is field-equivalent to the Kustomize bundles on all critical fields. The charts are hand-authored Go templates (research R3), NOT generated from Kustomize.

**Independent Test**: `helm lint` passes on both charts; `helm template` output matches `kustomize build` output on all critical fields (Kustomize↔Helm parity, data-model §2).

### Validation — write the cross-format parity check first

- [ ] T019 [P] [US1] Create a Kustomize↔Helm parity script at `.specify/scripts/bash/verify-cross-format-parity.sh` that renders `kustomize build deploy/kustomize/webhook` and `helm template deploy/charts/webhook`, groups resources by `kind:metadata.name`, and compares critical fields (data-model §2). Same for the equalizer. Exit non-zero on mismatch.

### Webhook chart

- [ ] T020 [P] [US1] Create `deploy/charts/webhook/Chart.yaml` per contracts/helm-chart.md §2.1 (apiVersion v2, name `emergency-ration-webhook`, type application, version `0.0.0-dev`, appVersion `0.0.0-dev`)
- [ ] T021 [P] [US1] Create `deploy/charts/webhook/values.yaml` per contracts/helm-chart.md §3.1 (image.repository `aectann/emergency-ration-webhook`, image.tag `latest`, image.pullPolicy `Always`, namespace `capacity-admission`, replicas 2, budget.defaultPercent 80, resources matching deploy defaults, certManager.enabled true)
- [ ] T022 [P] [US1] Create `deploy/charts/webhook/templates/_helpers.tpl` with named templates: `webhook.name`, `webhook.labels`, `webhook.namespace` (per contracts/helm-chart.md §5)
- [ ] T023 [P] [US1] Create `deploy/charts/webhook/templates/namespace.yaml` — Namespace `{{ .Values.namespace }}` (W1)
- [ ] T024 [P] [US1] Create `deploy/charts/webhook/templates/crds.yaml` — ClusterCapacity + Allocation CRDs (W4–W5). CRDs are cluster-scoped and should NOT be templated with namespace; copy the schema verbatim from deploy/kustomize/webhook/crds.yaml
- [ ] T025 [P] [US1] Create `deploy/charts/webhook/templates/deployment.yaml` — Deployment (W2) with image `{{ .Values.image.repository }}:{{ .Values.image.tag }}`, replicas `{{ .Values.replicas }}`, resources from `.Values.resources`, ports (8443 webhook, 9090 metrics), probes (/healthz on metrics port), volumeMount /tls, securityContext (runAsNonRoot true, runAsUser 65532, readOnlyRootFilesystem true, capabilities drop ALL). All non-parameterized fields hardcoded to match data-model §2.3
- [ ] T026 [P] [US1] Create `deploy/charts/webhook/templates/service.yaml` — Service `{{ template "webhook.name" . }}` (W3) with selector app label, ports webhook (8443→webhook) + metrics (9090→metrics)
- [ ] T027 [P] [US1] Create `deploy/charts/webhook/templates/rbac.yaml` — ServiceAccount + ClusterRole + ClusterRoleBinding (W6–W8). Preserve the exact RBAC verb lists from data-model §2.2 verbatim in the template
- [ ] T028 [P] [US1] Create `deploy/charts/webhook/templates/webhook-config.yaml` — ValidatingWebhookConfiguration (W9) with failurePolicy Fail, sideEffects None, timeoutSeconds 5, matchPolicy Exact, namespaceSelector NotIn `{{ .Values.namespace }}`. Service reference uses `{{ template "webhook.name" . }}` + `{{ .Values.namespace }}`. Preserve the cert-manager annotation `cert-manager.io/inject-ca-from`
- [ ] T029 [P] [US1] Create `deploy/charts/webhook/templates/cert-setup.yaml` — cert-manager Issuer + Certificate (W10–W11), wrapped in `{{- if .Values.certManager.enabled }}`. dnsNames use `{{ template "webhook.name" . }}` + namespace variants

### Equalizer chart

- [ ] T030 [P] [US1] Create `deploy/charts/equalizer/Chart.yaml` per contracts/helm-chart.md §2.2 (name `emergency-ration-equalizer`, version `0.0.0-dev`)
- [ ] T031 [P] [US1] Create `deploy/charts/equalizer/values.yaml` per contracts/helm-chart.md §3.2 (image.repository `aectann/emergency-ration-equalizer`, image.tag `latest`, namespace `capacity-equalizer`, reconcile.intervalSeconds 10, resources matching deploy defaults)
- [ ] T032 [P] [US1] Create `deploy/charts/equalizer/templates/_helpers.tpl` with `equalizer.name`, `equalizer.labels`, `equalizer.namespace`
- [ ] T033 [P] [US1] Create `deploy/charts/equalizer/templates/namespace.yaml` — Namespace `{{ .Values.namespace }}` (E1)
- [ ] T034 [P] [US1] Create `deploy/charts/equalizer/templates/crds.yaml` — EqualizerConfig CRD (E3), schema verbatim from deploy/kustomize/equalizer/crds.yaml
- [ ] T035 [P] [US1] Create `deploy/charts/equalizer/templates/deployment.yaml` — Deployment (E2) with image from values, env EQUALIZER_RECONCILE_INTERVAL_SECS `{{ .Values.reconcile.intervalSeconds }}`, securityContext matching data-model §2.3
- [ ] T036 [P] [US1] Create `deploy/charts/equalizer/templates/rbac.yaml` — ServiceAccount + ClusterRole + ClusterRoleBinding (E4–E6). Preserve the target-cluster RBAC comment block from deploy/kustomize/equalizer/rbac.yaml as a YAML comment in the template
- [ ] T037 [P] [US1] Create `deploy/charts/equalizer/templates/equalizer-config.example.yaml` — the example EqualizerConfig + kubeconfig Secrets from deploy/kustomize/equalizer/example-config.yaml, wrapped entirely in a YAML comment (`#`) or guarded by a `{{- if false }}` block so it is NOT rendered by default (it is documentation)

### Validate

- [ ] T038 [US1] Run `helm lint deploy/charts/webhook` and `helm lint deploy/charts/equalizer` — both must pass with zero errors
- [ ] T039 [US1] Run the cross-format parity script (T019) and verify zero mismatches between Helm-rendered and Kustomize-rendered output for both components

**Checkpoint**: Both Helm charts exist, lint-clean, and produce field-equivalent resources to the Kustomize bundles.

---

## Phase 4: Internal Consumer Migration (US4)

**Goal**: Migrate `erw-verify` and both CI workflows from the raw manifests to the Kustomize bundles. This is the precondition for deleting the raw files. Highest-risk phase — `erw-verify` changes are Rust source code.

**Independent Test**: `cargo build --bin erw-verify` succeeds (requires `kustomize`/`kubectl` on PATH); `erw-verify` S1–S11 scenarios pass against a kind cluster; both CI E2E workflows pass.

### erw-verify build.rs (research R5, contracts/erw-verify-migration.md)

- [ ] T040 [US4] Create `src/bin/erw-verify/build.rs` that runs `kustomize build deploy/kustomize/webhook` (falling back to `kubectl kustomize` if `kustomize` binary is absent), writes the rendered YAML to `$OUT_DIR/webhook-manifests.yaml`, and sets `cargo:rerun-if-changed=deploy/kustomize/webhook`. Assert non-zero exit on failure with a clear panic message (contracts/erw-verify-migration.md §2.1)
- [ ] T041 [US4] Modify `src/bin/erw-verify/setup.rs`: replace the four `include_str!` constants (CRDS, RBAC, DEPLOYMENT, WEBHOOK_CONFIG pointing at `../../../deploy/*.yaml`) with a single `const WEBHOOK_MANIFESTS: &str = include_str!(concat!(env!("OUT_DIR"), "/webhook-manifests.yaml"))`. The `apply_manifests` function already splits multi-document YAML via `serde_yaml::Deserializer` — verify it handles the Kustomize-rendered stream correctly (contracts/erw-verify-migration.md §2.2)
- [ ] T042 [US4] Modify the image substitution logic in `src/bin/erw-verify/setup.rs` `apply_manifests` (or the `deployment_doc` helper): change the find-and-replace target from `ERW_IMAGE_PLACEHOLDER` to the Kustomize default image reference (`capacity-admission-webhook:latest` or the resolved name from the kustomization images directive). The `.env`-driven image resolution (env_config.rs) is unchanged — only the substitution target string changes (contracts/erw-verify-migration.md §3)
- [ ] T043 [US4] Update the unit test in `src/bin/erw-verify/setup.rs` (line ~453, `deployment_doc("ERW_IMAGE_PLACEHOLDER")`) to use the new image reference `deployment_doc("capacity-admission-webhook:latest")` or the kustomization-resolved name. Verify the test asserts the image is correctly found and replaced in the Deployment document

### CI workflow migration (research R8, contracts/erw-verify-migration.md §4)

- [ ] T044 [US4] Modify `.github/workflows/ci.yml` E2E job: replace the `sed -e 's|image: ERW_IMAGE_PLACEHOLDER|image: capacity-admission-webhook:e2e|' deploy/deployment.yaml | kubectl apply -f -` pattern with `kubectl kustomize deploy/kustomize/webhook | sed 's|capacity-admission-webhook:latest|capacity-admission-webhook:e2e|' | kubectl apply -f -`. Apply crds/rbac/webhook-config via the same kustomize render (the kustomization bundles all resources). The caBundle sed injection on webhook-config may need to target the rendered output instead of the file path — verify the `sed "s|# caBundle: .*|caBundle: ${CABUNDLE}|"` piped-apply pattern still works on rendered output
- [ ] T045 [US4] Modify `.github/workflows/ci.yml`: replace `kubectl apply -f deploy/rbac.yaml` and `kubectl apply -f deploy/crds.yaml` with the kustomize-rendered equivalents (they are now part of `kustomize build` output, so a single `kubectl kustomize deploy/kustomize/webhook | kubectl apply -f -` with the image sed replaces the three separate apply steps)
- [ ] T046 [US4] Modify `.github/workflows/equalizer-e2e.yml`: replace all `kubectl apply -f deploy/equalizer/{crds,rbac}.yaml` and the `sed ... deploy/equalizer/deployment.yaml` pattern with `kubectl kustomize deploy/kustomize/equalizer | sed 's|capacity-equalizer:latest|capacity-equalizer:e2e|' | kubectl apply -f -`. Also migrate the webhook stack deploy in this workflow (lines ~114–127) to the kustomize pattern from T044
- [ ] T047 [US4] Add a `kubectl kustomize` (or `kustomize`) availability check to both CI workflows — `kubectl kustomize` is bundled in kubectl, so no separate install step should be needed, but verify the kind-action kubectl version supports it

### Validate

- [ ] T048 [US4] Run `cargo build --bin erw-verify` — verify the build.rs successfully renders Kustomize and the binary compiles with the embedded manifests
- [ ] T049 [US4] Run `cargo test` (unit + integration + BDD) — verify zero regressions. Integration tests use mocked-apiserver fixtures and do NOT consume deploy manifests (FR-020), so they must pass unchanged
- [ ] T050 [US4] Verify the `erw-verify` unit tests for `setup.rs` (the `deployment_doc` image substitution test) pass with the new image reference

### Delete raw manifests (after all consumers migrated)

- [ ] T051 [US4] Delete the raw manifest files per data-model §5: `deploy/crds.yaml`, `deploy/deployment.yaml`, `deploy/rbac.yaml`, `deploy/webhook-config.yaml`, `deploy/cert-setup.yaml`, `deploy/equalizer/crds.yaml`, `deploy/equalizer/deployment.yaml`, `deploy/equalizer/rbac.yaml`, `deploy/equalizer/equalizer-config.example.yaml`, and the empty `deploy/equalizer/` directory
- [ ] T052 [US4] Run `grep -rn 'ERW_IMAGE_PLACEHOLDER\|ERW_EQUALIZER_IMAGE_PLACEHOLDER\|deploy/deployment.yaml\|deploy/rbac.yaml\|deploy/crds.yaml\|deploy/webhook-config.yaml\|deploy/cert-setup.yaml\|deploy/equalizer/' src/ .github/ docs/ README.md CONTRIBUTING.md` and verify zero matches remain (the raw paths and placeholder tokens are fully purged)

**Checkpoint**: All consumers migrated. Raw manifests deleted. The repository has a single manifest source of truth (Kustomize) + a parameterized packaging (Helm).

---

## Phase 5: Release Workflow — Chart Packaging (US3)

**Goal**: Extend the `publish.yml` workflow to package both Helm charts and attach the `.tgz` files to the GitHub Release on tag push.

**Independent Test**: A tag push produces a GitHub Release with two `.tgz` attachments, each passing `helm lint`, with the chart version matching the tag.

- [ ] T053 [P] [US3] Add a `charts` job to `.github/workflows/publish.yml` per contracts/release-workflow.md §2: `needs: quality`, runs-on ubuntu-latest, no `if` repo gate needed (or mirror the publish job's gate)
- [ ] T054 [P] [US3] Add the version-stamping step (contracts/release-workflow.md §3): extract `VERSION="${GITHUB_REF_NAME#v}"`, fall back to `0.0.0-dev-<short-sha>` for workflow_dispatch, `sed -i` the `version:` and `appVersion:` fields in both `Chart.yaml` files
- [ ] T055 [P] [US3] Add the `helm lint` gate step (contracts/release-workflow.md §4): lint both charts; a failing lint exits the job non-zero, blocking the release
- [ ] T056 [P] [US3] Add the `helm package` step: package both charts to `.temp/charts/` (or a CI temp dir). Verify the output filenames are `emergency-ration-webhook-<version>.tgz` and `emergency-ration-equalizer-<version>.tgz`
- [ ] T057 [P] [US3] Add the release-attachment step using `softprops/action-gh-release@v2` (contracts/release-workflow.md §5): attach both `.tgz` files to the GitHub Release. This uses the default `GITHUB_TOKEN` — no additional secrets needed
- [ ] T058 [US3] Verify the `charts` job runs parallel to the `publish` (Docker) job, both gated on `quality`, and that neither depends on the other

**Checkpoint**: A tag push builds Docker images AND packages + attaches Helm charts.

---

## Phase 6: Documentation & Governance (US5)

**Goal**: Update every documentation surface to reference the new bundles, create the manifest-bundles article, expand ARTIFACTS.md, and amend the constitution.

### Constitution amendment (research R12)

- [ ] T059 [US5] Amend `.specify/memory/constitution.md`: add Principle XVI (Manifest Bundle Release Artifacts) — every containerised artifact in `ARTIFACTS.md` MUST ship Kustomize + Helm bundles as release artifacts, same same-change obligation as Principle XV. Bump version 2.9.0 → 2.10.0 (MINOR — new principle). Update the Sync Impact Report header, Added principles list, Added sections (Core Principles I–XVI), Development Workflow (add manifest-bundle obligation bullet), and the version/date stamps per the amendment procedure

### ARTIFACTS.md (FR-015)

- [ ] T060 [US5] Add a "Manifest bundles" section to `ARTIFACTS.md` per contracts/release-workflow.md §8: table with columns Component | Kustomize path | Helm chart name | Release artifact format. Document the webhook + equalizer entries. Link to `docs/manifest-bundles.md` for installation instructions

### New docs article (FR-021)

- [ ] T061 [P] [US5] Create `docs/manifest-bundles.md` documenting installation via both Kustomize and Helm with copy-pasteable commands: (1) Helm: download `.tgz` from GitHub Release, `helm install` with image/namespace/budget values, values reference table from contracts/helm-chart.md §3; (2) Kustomize: `kustomize build deploy/kustomize/<component>` with `kustomize edit set image`, the image override mechanism from contracts/kustomize-bundle.md §3. Include the EqualizerConfig example reference (applied separately from the chart)

### README updates (FR-022, FR-025)

- [ ] T062 [US5] Update `README.md` Quick Start: replace the `kubectl apply -f deploy/deployment.yaml` 6-step sequence with a Helm-first install (`helm install`) and a Kustomize alternative (`kustomize build deploy/kustomize/webhook | kubectl apply -f -`). Update the Published Image section to reference the chart `.tgz` as the install artifact. Keep it concise (hub model — link to docs/manifest-bundles.md for detail)
- [ ] T063 [US5] Add `docs/manifest-bundles.md` to the `README.md` Table of Contents (Documentation section) with a 1–3 sentence summary per Principle X
- [ ] T064 [US5] Update the `README.md` deploy tree code block (if it lists `deploy/` contents) to show `kustomize/` + `charts/` instead of the raw `.yaml` files

### docs/deployment.md (FR-023)

- [ ] T065 [US5] Update `docs/deployment.md`: rewrite every reference to `deploy/deployment.yaml`, `deploy/rbac.yaml`, `deploy/crds.yaml`, `deploy/webhook-config.yaml`, `deploy/cert-setup.yaml` to the Kustomize bundle paths or Helm chart install commands. Update the Published Image section, Build the Image section (image override is now via kustomize edit set image or helm values), and the TLS Provisioning section (paths)

### CONTRIBUTING.md (FR-024)

- [ ] T066 [US5] Update `CONTRIBUTING.md`: (1) add `kustomize` (or `kubectl` with kustomize support) to the prerequisites for building `erw-verify` (contracts/erw-verify-migration.md §5); (2) update the Container Image section to reference `deploy/kustomize/webhook/` for the image override; (3) add a Chart Packaging subsection under Publishing/ Releases documenting the `charts` job in publish.yml (version stamping, helm lint gate, helm package, GitHub Release attachment); (4) update the Project Structure source tree to show `deploy/kustomize/` + `deploy/charts/` instead of the raw yaml files
- [ ] T067 [US5] Update the `CONTRIBUTING.md` verification tool section: the `erw-verify` binary now embeds Kustomize-rendered manifests at compile time via build.rs — note the build-time `kustomize` requirement

### AGENTS.md / CLAUDE.md

- [ ] T068 [US5] Update `AGENTS.md` and `CLAUDE.md` project-description sections if they reference the raw `deploy/*.yaml` paths (the SPECKIT block already points at the spec-015 plan; check the project description blurb)

### Documentation sweep validation (FR-023)

- [ ] T069 [US5] Run `grep -rn 'deploy/deployment.yaml\|deploy/rbac.yaml\|deploy/crds.yaml\|deploy/webhook-config.yaml\|deploy/cert-setup.yaml\|deploy/equalizer/' README.md docs/ CONTRIBUTING.md .github/workflows/` and verify zero matches remain (quickstart.md V12)

---

## Phase 7: Polish & CI Integration

**Purpose**: Wire the parity/validation scripts into CI and run the full quickstart validation.

- [ ] T070 [P] Add a `helm-lint` job (or step in the existing quality job) to `.github/workflows/ci.yml`: install helm, run `helm lint` on both charts. Gate: failure blocks PR merge (FR-006)
- [ ] T071 [P] Add the Kustomize↔pre-migration parity check (T004/T005 scripts) to `ci.yml` as a validation step — after the raw manifests are deleted (Phase 4), this script compares Kustomize output against a snapshot OR is converted to a Kustomize↔Helm cross-format parity check (T019 script). Decide based on whether the raw manifests still exist at CI time
- [ ] T072 Run `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — verify the full quality gate passes (the build.rs and setup.rs changes must be clippy-clean)
- [ ] T073 Run the quickstart.md validation scenarios (V1–V12) locally or in CI: helm install, kustomize build+apply, chart packaging, erw-verify scenarios, documentation sweep
- [ ] T074 Run `editorconfig-checker` to verify all new YAML/TPL files comply with `.editorconfig` (Principle IX)
- [ ] T075 Verify the `.specify/feature.json` still points to `specs/015-kustomize-helm-manifests` and the agent-context SPECKIT blocks in AGENTS.md/CLAUDE.md point to the spec-015 plan.md

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — create directories + scaffold first
- **Phase 2 (Kustomize — US2)**: Depends on Phase 1. BLOCKS all other phases. This is the manifest source of truth.
- **Phase 3 (Helm — US1)**: Depends on Phase 2 (Kustomize bundles must exist for cross-format parity validation)
- **Phase 4 (Consumer Migration — US4)**: Depends on Phase 2 (Kustomize bundles must exist for erw-verify build.rs + CI migration). Can run parallel to Phase 3.
- **Phase 5 (Release Workflow — US3)**: Depends on Phase 3 (Helm charts must exist to package). Can run parallel to Phase 4.
- **Phase 6 (Documentation — US5)**: Depends on Phases 2–5 (document what was built). Runs after all code/infra changes.
- **Phase 7 (Polish)**: Depends on all prior phases.

### User Story Dependencies

- **US2 (Kustomize)**: Foundational — no dependencies on other stories. Implemented first.
- **US1 (Helm)**: Depends on US2 (parity validation target).
- **US4 (Migration)**: Depends on US2 (Kustomize bundles are the migration target).
- **US3 (Release)**: Depends on US1 (charts must exist to package).
- **US5 (Documentation)**: Depends on all prior stories (document the finished artifacts).

### Parallel Opportunities

- Phase 2: T004–T016 are mostly [P] (independent files within the bundle)
- Phase 3: T019–T037 are mostly [P] (independent template files)
- Phase 4: T040–T043 (erw-verify) must be sequential (same module, build dependency chain). T044–T047 (CI workflows) are [P] after erw-verify is done.
- Phase 5: T053–T058 are [P] (single workflow file, but logically independent steps)
- Phase 6: T059 (constitution) → T060 (ARTIFACTS.md) → T061–T068 (docs, mostly [P])

---

## Implementation Strategy

### MVP First (US2 only)

1. Complete Phase 1: Setup (directories + scaffold)
2. Complete Phase 2: Kustomize bundles + parity validation
3. **STOP and VALIDATE**: `kustomize build` produces correct output; parity check passes
4. At this point the manifest source of truth exists; all downstream work can proceed

### Incremental Delivery

1. Setup → Kustomize bundles (source of truth)
2. Add Helm charts → validate cross-format parity
3. Migrate erw-verify + CI → delete raw manifests
4. Add chart packaging to release workflow
5. Update all documentation + constitution
6. Wire CI gates + final validation

---

## Notes

- **`[P]` tasks** = different files, no dependencies on incomplete tasks
- **Story label** maps task to specific user story for traceability
- **Verification-only tasks** (T049, T050, FR-020): integration tests use mocked-apiserver fixtures and do NOT consume deploy manifests — assert no regression, do not invent test changes
- **build.rs kustomize dependency**: T040 introduces a build-time requirement for `kustomize`/`kubectl`. Document this in CONTRIBUTING.md (T066) and verify CI has it (T047)
- **Image substitution target change**: the `ERW_IMAGE_PLACEHOLDER` token disappears entirely. `erw-verify` finds `capacity-admission-webhook:latest` (the kustomization default) and replaces it. CI uses the rendered-output sed pattern.
- Commit after each phase or logical task group
- Stop at any checkpoint to validate independently
