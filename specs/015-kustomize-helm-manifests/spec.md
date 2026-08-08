# Feature Specification: Kustomize + Helm Manifest Bundles

**Feature Branch**: `spec/015-kustomize-helm-manifests`

**Created**: 2026-08-08

**Status**: Draft

**Input**: User description: "supply our users not only with docker artifacts
but also with verified manifest sets templated by 2 instruments Kustomize and
Helm. Kustomize and Helm bundles ready and presented as release artifacts."

## User Scenarios & Testing *(mandatory)*

The repository currently ships raw Kubernetes manifests (`deploy/*.yaml`,
`deploy/equalizer/*.yaml`) and Docker images. This feature adds two templated
manifest bundles — Kustomize and Helm — as first-class release artifacts, makes
Kustomize the single manifest source of truth (replacing the raw YAML), and
migrates all manifest consumers (the `erw-verify` tool, CI workflows, and
documentation) onto the new bundles. Every user story is independently
testable.

### User Story 1 — Operator installs via Helm chart (Priority: P1)

An operator who has pulled a published Docker image wants to install the
webhook (or the equalizer) into a cluster using Helm, overriding the image
reference and budget defaults via values — no `sed`, no manual placeholder
editing, no raw YAML surgery. On each tagged release, a versioned `.tgz` chart
package is attached to the GitHub Release. The operator downloads the
attachment and runs `helm install`.

**Why this priority**: Helm is the most common installation method for
Kubernetes operators. It is the headline user-facing deliverable of this
feature — the operator no longer needs to hand-edit manifests. Without it the
"verified manifest sets" promise is unfulfilled.

**Independent Test**: Download the `.tgz` from a release, run `helm install`
against a `kind` cluster with a values file pointing at a locally-loaded image,
and assert the webhook Deployment reaches Ready and enforces the budget (an
over-budget pod is rejected).

**Acceptance Scenarios**:

1. **Given** a tagged release `vX.Y.Z` on GitHub, **When** the operator
   downloads the `emergency-ration-webhook-<version>.tgz` attachment, **Then**
   `helm install erw ./emergency-ration-webhook-<version>.tgz` succeeds against
   a cluster and the webhook Deployment reaches Ready.
2. **Given** the same `.tgz`, **When** the operator sets
   `image.repository` / `image.tag` via a values file or `--set`, **Then** the
   webhook pod runs the specified image (no manual placeholder editing).
3. **Given** the equalizer chart `.tgz`, **When** installed with target-cluster
   kubeconfig Secrets pre-created, **Then** the equalizer Deployment reaches
   Ready and an `EqualizerConfig` singleton can be applied and reconciled.
4. **Given** a chart rendered with `helm template`, **When** the output is
   compared against the Kustomize-rendered manifests for the same component,
   **Then** the resource kinds, names, namespaces, and critical fields
   (failurePolicy, sideEffects, RBAC verbs, image reference) are identical.
5. **Given** the chart is linted, **When** `helm lint` runs, **Then** zero
   errors are reported.

---

### User Story 2 — Operator deploys via Kustomize (Priority: P1)

An operator who prefers Kustomize (or whose platform standardizes on it) wants
to deploy the webhook or equalizer from the repository's bundled Kustomize
base, overriding the image reference with `kustomize edit set image` or an
overlay — no raw `deploy/*.yaml` at the repo root, no `ERW_IMAGE_PLACEHOLDER`
sed pattern.

**Why this priority**: Kustomize is the second headline deliverable and is also
the **new source of truth** that the Helm charts and all internal consumers are
built on. It must land in the same release as the Helm charts because the raw
manifests it replaces are being deleted.

**Independent Test**: From a clone at a tagged commit, run `kustomize build
deploy/kustomize/webhook` with an image override, pipe to `kubectl apply`, and
assert the webhook reaches Ready and enforces the budget.

**Acceptance Scenarios**:

1. **Given** the repository at a tagged release, **When** the operator runs
   `kustomize build deploy/kustomize/webhook | kubectl apply -f -`, **Then** the
   webhook Namespace, Deployment, Service, CRDs, RBAC, and
   ValidatingWebhookConfiguration are all created and the Deployment reaches
   Ready.
2. **Given** an overlay that sets the image, **When** the operator runs
   `kustomize edit set image` (or applies an image patch), **Then** the rendered
   Deployment carries the specified image reference.
3. **Given** the equalizer Kustomize bundle at `deploy/kustomize/equalizer`,
   **When** built and applied, **Then** the equalizer namespace, Deployment,
   CRD, and RBAC are created and the Deployment reaches Ready.
4. **Given** the Kustomize-rendered webhook manifests, **When** compared
   field-by-field against the pre-migration raw `deploy/*.yaml` content, **Then**
   every resource kind, name, namespace, label, annotation, RBAC verb, webhook
   `failurePolicy`/`sideEffects`/`timeoutSeconds`, and container field is
   functionally identical (the only permitted differences are the image field,
   which is now templated, and the addition of kustomization config files).

---

### User Story 3 — Release ships manifest bundles alongside Docker images (Priority: P1)

A maintainer cuts a release by pushing a semver git tag. Today this triggers a
Docker image build+push. After this feature, the same tag push must additionally
package the two Helm charts and attach the `.tgz` files to the GitHub Release
created by the tag. The Kustomize bundles are shipped in-repo (consumed from a
tagged commit), so they need no separate packaging — but the release notes and
artifact inventory must mention them.

**Why this priority**: The release procedure is the mechanism that actually
delivers the bundles to users. Without wiring the packaging into the tag-push
trigger, the charts exist in the repo but never reach a user who isn't cloning.
This is the "presented as release artifacts" half of the request.

**Independent Test**: Trigger the release workflow on a pre-release tag, then
`gh release view <tag>` and assert two `.tgz` attachments are present with the
correct version in their filenames, and that each `.tgz` passes `helm lint`.

**Acceptance Scenarios**:

1. **Given** a semver tag `vX.Y.Z` is pushed, **When** the release workflow
   completes, **Then** the GitHub Release for that tag has two attachments:
   `emergency-ration-webhook-<version>.tgz` and
   `emergency-ration-equalizer-<version>.tgz`.
2. **Given** a pre-release tag `vX.Y.Z-rc.N` is pushed, **When** the workflow
   completes, **Then** both `.tgz` attachments are present with the pre-release
   version in their filenames.
3. **Given** a downloaded chart `.tgz` from a release, **When** extracted and
   linted (`helm lint`), **Then** zero errors.
4. **Given** the chart `version` field inside the packaged `.tgz`, **When**
   inspected, **Then** it matches the git tag version (without the leading `v`).
5. **Given** `ARTIFACTS.md`, **When** read, **Then** it documents the Helm
   charts and Kustomize bundles as release artifacts alongside the Docker
   images, with their location and how to obtain them.

---

### User Story 4 — Internal consumers migrated onto Kustomize (Priority: P2)

The `erw-verify` verification tool and the CI E2E workflows currently consume
the raw root manifests. Once the raw files are deleted, these consumers must
operate against the Kustomize-rendered output instead — otherwise they break
the moment the root YAML is removed. This story is the migration that makes the
deletion in US2 safe.

**Why this priority**: P2 because it is invisible to the external user but is
the technical precondition for deleting the raw manifests (US2's "replace, not
parallel"). It must land in the same release; it just isn't the headline
deliverable.

**Independent Test**: Delete the raw `deploy/*.yaml` files, rebuild `erw-verify`,
and run its scenario suite against a `kind` cluster — all scenarios pass. Run
both CI E2E workflows and assert the webhook and equalizer stacks deploy and
the scenarios pass.

**Acceptance Scenarios**:

1. **Given** the raw `deploy/*.yaml` and `deploy/equalizer/*.yaml` are removed
   from the repository, **When** `erw-verify` is built and its S1–S11 scenarios
   are run against a `kind` cluster, **Then** all scenarios pass (the webhook
   stack deploys and the budget is enforced identically to pre-migration).
2. **Given** the equalizer E1–E5 scenarios, **When** run with target-cluster
   kubeconfigs against `kind` clusters, **Then** all pass (the equalizer stack
   deploys and reconciles identically to pre-migration).
3. **Given** the `ci.yml` E2E job, **When** it runs on a PR, **Then** it deploys
   the webhook via the Kustomize bundle (image overridden to the locally-loaded
   `kind` image) and the enforcement scenarios pass.
4. **Given** the `equalizer-e2e.yml` job, **When** it runs, **Then** it deploys
   both webhook and equalizer stacks via the Kustomize bundles and the
   cross-cluster scenarios pass.
5. **Given** `erw-verify` no longer contains any `include_str!` reference to a
   path under `deploy/*.yaml` or `deploy/equalizer/*.yaml`, **When** the source
   is grepped, **Then** zero matches remain.

---

### User Story 5 — Documentation reflects the new bundles (Priority: P2)

Every documentation surface that references the old raw manifest paths —
README quick-start, `docs/deployment.md`, `CONTRIBUTING.md` build/publish
sections — must be rewritten to point at the Kustomize and Helm bundles. A new
`docs/` article covers installation via both Kustomize and Helm. The
constitution's artifact principle is extended to cover manifest bundles.

**Why this priority**: P2 because documentation must ship with the feature
(constitutional same-change rule, Principle X), but it follows the bundles
being real rather than preceding them. The user explicitly called out
"discreet attention to alterations that must be made to the release procedure
and documentation."

**Independent Test**: Grep every documentation file for references to the
deleted root manifest paths and assert zero matches; assert the new
installation article exists and is linked from the README TOC.

**Acceptance Scenarios**:

1. **Given** README.md, **When** scanned, **Then** the Quick Start references
   `helm install` and/or `kustomize build deploy/kustomize/webhook`, not
   `kubectl apply -f deploy/deployment.yaml`.
2. **Given** `docs/deployment.md`, **When** read, **Then** every path points at
   the Kustomize/Helm bundles; no reference to `deploy/deployment.yaml` (root)
   remains.
3. **Given** `CONTRIBUTING.md`, **When** read, **Then** the build/test/publish
   sections describe chart packaging and the Kustomize-based deploy, and the
   publishing section documents the `.tgz` attachment step.
4. **Given** a new `docs/manifest-bundles.md` (or similarly named) article,
   **When** read, **Then** it documents both installation paths (Kustomize and
   Helm) with copy-pasteable commands, values tables, and the image-override
   mechanism.
5. **Given** the README Table of Contents, **When** scanned, **Then** the new
   article is listed with a 1–3 sentence summary.
6. **Given** `ARTIFACTS.md`, **When** read, **Then** it lists the Helm charts
   and Kustomize bundles as release artifacts with their location and how to
   obtain them.

---

### Edge Cases

- **What if the Kustomize-rendered output diverges from the pre-migration raw
  YAML?** A field-level comparison (US2 AC4) must catch any drift. The only
  permitted differences are the templated image field and the presence of
  kustomization config. Any RBAC verb, webhook failurePolicy, namespace, or
  resource-name divergence is a blocking defect — the Kustomize bundle MUST
  produce functionally identical resources.
- **What if a chart value is not set?** The chart's `values.yaml` MUST ship
  sensible defaults (the published Docker Hub image reference, the standard
  namespaces, the 80% default budget) so an install with no overrides produces a
  working deployment. A bare `helm install` with no values must succeed.
- **What if `helm lint` fails on the packaged `.tgz`?** The release workflow
  MUST run `helm lint` as a gate before attaching the chart; a failing lint
  blocks the release (CI-green completion gate, Principle XI).
- **What if `erw-verify` runs in an environment without the `kustomize` binary
  on PATH?** The tool currently embeds manifests at compile time. If it is
  reworked to render at runtime it gains a binary dependency; the design must
  address this (the plan phase resolves whether to embed pre-rendered output vs
  shell out to `kustomize build` vs render in-process).
- **What if the equalizer chart is installed without target-cluster kubeconfig
  Secrets?** The Deployment reaches Ready but the reconcile loop reports
  `Unreachable` for every target — this is existing behavior and MUST be
  preserved, not changed.
- **What about the `ERW_IMAGE_PLACEHOLDER` / `ERW_EQUALIZER_IMAGE_PLACEHOLDER`
  tokens?** These sed-target tokens disappear from the repo entirely. The
  Kustomize image field and the Helm `image.repository`/`image.tag` values
  replace them. The `.env`-driven substitution in `erw-verify` must be reworked
  to target the new mechanism.
- **What if the same component is installed twice (e.g. webhook in two
  namespaces)?** Helm supports this natively (release name + namespace). The
  chart MUST not hardcode the namespace in a way that prevents a second install;
  `namespace` must be a value, not a literal in every template. Kustomize
  namespace is set via `namespace:` transformer.

## Requirements *(mandatory)*

### Functional Requirements

**Helm charts (US1)**

- **FR-001**: The repository MUST contain two Helm charts:
  `deploy/charts/webhook/` (chart name `emergency-ration-webhook`) and
  `deploy/charts/equalizer/` (chart name `emergency-ration-equalizer`).
- **FR-002**: Each chart MUST template every resource currently in the
  corresponding raw manifest set: the webhook chart covers Namespace,
  Deployment, Service, ClusterCapacity CRD, Allocation CRD, RBAC
  (ServiceAccount + ClusterRole + ClusterRoleBinding),
  ValidatingWebhookConfiguration, and the cert-manager Certificate/Issuer; the
  equalizer chart covers Namespace, Deployment, EqualizerConfig CRD, and RBAC.
- **FR-003**: Each chart's `values.yaml` MUST expose the container image
  (`repository` + `tag`) so an operator can point at any registry/tag without
  editing templates.
- **FR-004**: Each chart MUST ship sensible defaults in `values.yaml` such that
  a bare `helm install` with no overrides produces a working deployment against
  the published Docker Hub image.
- **FR-005**: The webhook chart MUST expose the default budget percentage as a
  value (default 80).
- **FR-006**: Each chart MUST pass `helm lint` with zero errors.

**Kustomize bundles (US2)**

- **FR-007**: The repository MUST contain two Kustomize bundles:
  `deploy/kustomize/webhook/` and `deploy/kustomize/equalizer/`, each with a
  `kustomization.yaml`.
- **FR-008**: `kustomize build deploy/kustomize/webhook` MUST render
  functionally identical resources to the pre-migration raw `deploy/*.yaml` set
  (Namespace, Deployment, Service, CRDs, RBAC, ValidatingWebhookConfiguration,
  cert-manager resources) — identical kinds, names, namespaces, labels,
  annotations, RBAC verbs, webhook failurePolicy/sideEffects/timeoutSeconds,
  and container fields, differing only in the image field (now templated).
- **FR-009**: `kustomize build deploy/kustomize/equalizer` MUST render
  functionally identical resources to the pre-migration `deploy/equalizer/*.yaml`
  set.
- **FR-010**: Each Kustomize bundle MUST support image override via the
  standard `images:` directive (`kustomize edit set image ...`) so an operator
  or CI can point at any image without editing YAML.
- **FR-011**: The raw `deploy/*.yaml` and `deploy/equalizer/*.yaml` files MUST
  be deleted once FR-007–FR-010 and FR-014–FR-017 are satisfied. They are not
  kept as a parallel path.

**Release artifacts (US3)**

- **FR-012**: The tag-push release workflow MUST package both Helm charts into
  versioned `.tgz` archives and attach them to the GitHub Release for that tag.
- **FR-013**: The chart `version` field in each packaged `.tgz` MUST match the
  git tag (without the leading `v`).
- **FR-014**: The release workflow MUST run `helm lint` on each chart and block
  the release (fail the job) if lint fails.
- **FR-015**: `ARTIFACTS.md` MUST list the Helm charts and the Kustomize bundles
  as release artifacts alongside the Docker images, with their location and how
  to obtain them.

**Internal consumer migration (US4)**

- **FR-016**: `erw-verify` MUST consume the Kustomize-rendered manifests (not
  the deleted raw files) such that its S1–S11 webhook scenarios and E1–E5
  equalizer scenarios behave identically to pre-migration. The implementation
  MUST NOT retain `include_str!` references to any path under the deleted
  `deploy/*.yaml` or `deploy/equalizer/*.yaml`.
- **FR-017**: The `.env`-driven image substitution in `erw-verify` MUST target
  the new Kustomize image-override mechanism (replacing the
  `ERW_IMAGE_PLACEHOLDER` / `ERW_EQUALIZER_IMAGE_PLACEHOLDER` sed/replace
  approach).
- **FR-018**: The `ci.yml` E2E job MUST deploy the webhook via
  `kustomize build deploy/kustomize/webhook` (image overridden to the
  locally-loaded `kind` image) instead of `sed ... deploy/deployment.yaml`.
- **FR-019**: The `equalizer-e2e.yml` job MUST deploy both stacks via their
  Kustomize bundles instead of the raw manifests.
- **FR-020**: The integration test suite (`tests/`) MUST continue to pass
  unchanged — it consumes mocked-apiserver fixtures constructed in code, not
  the deploy manifests, so no integration test modification is expected. This
  requirement exists to assert the absence of regressions.

**Documentation (US5)**

- **FR-021**: A new `docs/manifest-bundles.md` article MUST document
  installation via both Kustomize and Helm, with copy-pasteable commands, a
  values reference table, and the image-override mechanism for each.
- **FR-022**: README.md Quick Start MUST reference the Helm and/or Kustomize
  install path, not `kubectl apply -f deploy/deployment.yaml`.
- **FR-023**: Every documentation surface (README, `docs/deployment.md`,
  `CONTRIBUTING.md`, CI workflow comments) MUST be free of references to the
  deleted root manifest paths (`deploy/deployment.yaml`, `deploy/rbac.yaml`,
  `deploy/crds.yaml`, `deploy/webhook-config.yaml`, `deploy/cert-setup.yaml`,
  `deploy/equalizer/*.yaml`).
- **FR-024**: `CONTRIBUTING.md` MUST document the chart-packaging release step
  and the Kustomize-based local deploy.
- **FR-025**: The README Table of Contents MUST link the new
  `docs/manifest-bundles.md` article with a 1–3 sentence summary.
- **FR-026**: The constitution MUST be amended with a new principle (or
  expansion of Principle XV) requiring that every containerised artifact also
  ship a versioned manifest bundle (Kustomize + Helm) as a release artifact,
  with the same same-change obligation as the Docker image publish procedure.

### Key Entities *(include if feature involves data)*

- **Kustomize bundle**: a directory (`deploy/kustomize/<component>/`) containing
  a `kustomization.yaml` plus the resource manifests and an image-overridable
  base. The single source of truth for what resources a component deploys.
- **Helm chart**: a directory (`deploy/charts/<component>/`) containing
  `Chart.yaml`, `values.yaml`, and `templates/` — a parameterized packaging of
  the same resources, distributed as a `.tgz` release attachment.
- **Release artifact inventory**: `ARTIFACTS.md`, extended to list manifest
  bundles (Kustomize path + Helm chart name + distribution mechanism) alongside
  Docker images, per the extended constitution principle.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can install a working, budget-enforcing webhook into
  a cluster in a single `helm install` command from a release `.tgz`, with no
  manual YAML editing, in under 2 minutes from download to Ready.
- **SC-002**: An operator can deploy a working webhook from a repository clone
  in a single `kustomize build ... | kubectl apply -f -` pipeline (with an image
  override), with no `sed` or placeholder editing.
- **SC-003**: A tagged release's GitHub Release page shows two `.tgz`
  attachments (webhook + equalizer charts), each passing `helm lint`, with the
  chart version matching the tag.
- **SC-004**: The Kustomize-rendered webhook manifests are field-by-field
  identical to the pre-migration raw `deploy/*.yaml` for every resource
  (permitted difference: the templated image field).
- **SC-005**: The full `erw-verify` scenario suite (S1–S11 + E1–E5) passes
  against `kind` clusters with the raw root manifests deleted — zero behavioral
  regression versus pre-migration.
- **SC-006**: Both CI E2E workflows (`ci.yml`, `equalizer-e2e.yml`) pass on a
  PR, deploying via the Kustomize bundles.
- **SC-007**: Zero documentation references to the deleted root manifest paths
  remain anywhere in the repository (README, docs/, CONTRIBUTING, CI comments).
- **SC-008**: `ARTIFACTS.md` and the constitution cover manifest bundles as
  first-class release artifacts with the same same-change obligation as Docker
  images.

## Assumptions

- The published Docker Hub images (`aectann/emergency-ration-webhook`,
  `aectann/emergency-ration-equalizer`) remain the default image references in
  chart `values.yaml` and Kustomize defaults; this feature does not change the
  image publishing itself, only adds manifest bundling on top.
- Helm and Kustomize are standard, widely-available operator tools; requiring
  one or both on the operator's PATH is acceptable (the chart `.tgz` path needs
  only `helm`; the Kustomize path needs `kustomize`).
- The existing `deploy/equalizer/equalizer-config.example.yaml` example
  (`EqualizerConfig` + kubeconfig `Secret`s) is migrated into the Kustomize
  equalizer bundle and the Helm chart's templates/values, not left behind as a
  raw file. The target-cluster RBAC comment block in `deploy/equalizer/rbac.yaml`
  is preserved as documentation inside the bundle.
- The integration test suite (`tests/integration/*`, `tests/bdd/*`,
  `tests/equalizer/*`, `tests/verify/*`) is unaffected because it builds
  in-code mocked-apiserver fixtures and does not consume the deploy manifests;
  this assumption is validated as a verification-only task (FR-020).
- The `.env`-driven build automation (spec-009) continues to drive
  `erw-verify`'s image resolution; only the substitution target changes (from a
  placeholder token in raw YAML to the Kustomize image directive / Helm value).
- Chart versioning follows the git tag: a tag `vX.Y.Z` produces chart version
  `X.Y.Z`; a pre-release `vX.Y.Z-rc.N` produces chart version `X.Y.Z-rc.N`.
- The release workflow attaches charts to the GitHub Release that GitHub
  automatically creates for the tag (or creates one if GitHub's auto-create is
  off); no separate GitHub Pages chart repository is stood up (per clarification
  — GitHub Release attachment only).
