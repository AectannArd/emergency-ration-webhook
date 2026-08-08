#!/usr/bin/env bash
# verify-manifest-parity.sh — Kustomize↔raw parity for the webhook bundle.
# spec-015, task T004 (Constitution Principle VIII: write the check first).
#
# Renders `kustomize build deploy/kustomize/webhook` and compares the resource
# set against the pre-migration raw manifests (deploy/{crds,deployment,rbac,
# webhook-config,cert-setup}.yaml), asserting field-level equivalence on EVERY
# field. The ONLY permitted difference is the container `image:` field
# (ERW_IMAGE_PLACEHOLDER in the raw → resolved by the kustomization `images:`
# directive in the bundle). Exits non-zero on any mismatch.
#
# Critical fields enforced (data-model §2): failurePolicy, sideEffects,
# timeoutSeconds, matchPolicy, admissionReviewVersions, RBAC verb lists,
# securityContext, container ports, probes, namespaces, names.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"

bundle="$repo_root/deploy/kustomize/webhook"
raw_files=(
  "$repo_root/deploy/crds.yaml"
  "$repo_root/deploy/deployment.yaml"
  "$repo_root/deploy/rbac.yaml"
  "$repo_root/deploy/webhook-config.yaml"
  "$repo_root/deploy/cert-setup.yaml"
)
component="webhook"

command -v kustomize >/dev/null 2>&1 || {
  echo "error: kustomize not found on PATH" >&2
  exit 2
}
command -v python3 >/dev/null 2>&1 || {
  echo "error: python3 not found on PATH" >&2
  exit 2
}

kustomized="$(kustomize build "$bundle")"
raw="$(cat "${raw_files[@]}")"

export PARITY_COMPONENT="$component"
python3 - "$kustomized" "$raw" <<'PY'
import os
import sys
import yaml


def load(text):
    return [d for d in yaml.safe_load_all(text) if d]


def key(d):
    m = d.get("metadata", {}) or {}
    return (d.get("apiVersion"), d.get("kind"), m.get("name"))


def strip_image(node):
    """Deep copy with every `image` key removed — the only permitted diff."""
    if isinstance(node, dict):
        return {k: strip_image(v) for k, v in node.items() if k != "image"}
    if isinstance(node, list):
        return [strip_image(x) for x in node]
    return node


def find_images(node, acc):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == "image":
                acc.append(v)
            else:
                find_images(v, acc)
    elif isinstance(node, list):
        for x in node:
            find_images(x, acc)


component = os.environ["PARITY_COMPONENT"]
kuz = load(sys.argv[1])
raw = load(sys.argv[2])

errors = []
kmap = {key(d): d for d in kuz}
raw_keys = {key(d) for d in raw}

for d in raw:
    k = key(d)
    if k not in kmap:
        errors.append("MISSING in kustomize output: %s" % (k,))
        continue
    if strip_image(kmap[k]) != strip_image(d):
        errors.append(
            "FIELD DRIFT for %s:\n  raw:       %s\n  kustomize: %s"
            % (k, strip_image(d), strip_image(kmap[k]))
        )
    # The image field must actually be resolved by the images directive.
    imgs = []
    find_images(kmap[k], imgs)
    for img in imgs:
        s = str(img)
        if "PLACEHOLDER" in s.upper() or s.endswith(":placeholder"):
            errors.append("image not resolved in kustomize output for %s: %s" % (k, img))

for k in set(kmap) - raw_keys:
    errors.append("EXTRA in kustomize output (not in raw): %s" % (k,))

if len(kuz) != len(raw):
    errors.append("resource count mismatch: kustomize=%d raw=%d" % (len(kuz), len(raw)))

if errors:
    print(
        "manifest parity FAILED — %s bundle vs raw manifests:" % component,
        file=sys.stderr,
    )
    for e in errors:
        print("  - " + e, file=sys.stderr)
    sys.exit(1)

print(
    "manifest parity OK — %s: %d resources match (image field permitted-diff "
    "verified resolved)." % (component, len(raw))
)
PY
