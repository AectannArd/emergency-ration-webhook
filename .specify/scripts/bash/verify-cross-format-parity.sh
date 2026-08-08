#!/usr/bin/env bash
# verify-cross-format-parity.sh — Kustomize↔Helm parity (spec-015, task T019).
#
# Renders `kustomize build` and `helm template` for both components and compares
# the resource sets field-by-field, grouped by (kind, metadata.name). Validates
# US1/US2 AC4 (Kustomize ↔ Helm equivalence) on the contract-critical fields of
# data-model §2 (failurePolicy, sideEffects, timeoutSeconds, matchPolicy,
# admissionReviewVersions, RBAC verb lists, securityContext, container ports,
# probes, selectors, CRD schemas). Exits non-zero on any mismatch.
#
# Usage: verify-cross-format-parity.sh [webhook|equalizer|both]
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"

command -v kustomize >/dev/null 2>&1 || {
  echo "error: kustomize not found on PATH" >&2
  exit 2
}
command -v helm >/dev/null 2>&1 || {
  echo "error: helm not found on PATH" >&2
  exit 2
}
command -v python3 >/dev/null 2>&1 || {
  echo "error: python3 not found on PATH" >&2
  exit 2
}

target="${1:-both}"

check_component() {
  local component="$1"
  local kustom_dir="$repo_root/deploy/kustomize/$component"
  local chart_dir="$repo_root/deploy/charts/$component"

  local kuz helm_out
  kuz="$(kustomize build "$kustom_dir")"
  # `--debug --is-upgrade` off; release name is irrelevant because templates use
  # fixed resource names. Include crds so CRDs render under the same scope.
  helm_out="$(helm template "$chart_dir" --include-crds)"

  PARITY_COMPONENT="$component" python3 - "$kuz" "$helm_out" <<'PY'
import os
import sys
import yaml


def load(text):
    return [d for d in yaml.safe_load_all(text) if d]


def key(d):
    m = d.get("metadata", {}) or {}
    return (d.get("kind"), m.get("name"))


def diff(a, b, path=""):
    """Recursive structural diff → list of (path, raw_a, raw_b) tuples."""
    out = []
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            p = f"{path}.{k}" if path else k
            if k not in a:
                out.append((p, "<missing>", a_repr(b[k])))
            elif k not in b:
                out.append((p, a_repr(a[k]), "<missing>"))
            else:
                out += diff(a[k], b[k], p)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append((path, a_repr(a), a_repr(b)))
        else:
            for i, (x, y) in enumerate(zip(a, b)):
                out += diff(x, y, f"{path}[{i}]")
    else:
        if a != b:
            out.append((path, a_repr(a), a_repr(b)))
    return out


def a_repr(v):
    return repr(v)


component = os.environ["PARITY_COMPONENT"]
kuz = load(sys.argv[1])
helm = load(sys.argv[2])

kmap = {key(d): d for d in kuz}
hmap = {key(d): d for d in helm}
errors = []

for k in sorted(set(kmap) | set(hmap), key=lambda t: (t[0] or "", t[1] or "")):
    if k not in kmap:
        errors.append("only in Helm output (not Kustomize): %s" % (k,))
        continue
    if k not in hmap:
        errors.append("only in Kustomize output (not Helm): %s" % (k,))
        continue
    for p, a, b in diff(kmap[k], hmap[k]):
        errors.append("  %s\n      kustomize=%s\n      helm=%s" % (str(k) + " :: " + p, a, b))

if len(kuz) != len(helm):
    errors.append(
        "resource count mismatch: kustomize=%d helm=%d" % (len(kuz), len(helm))
    )

if errors:
    print(
        "cross-format parity FAILED — %s (Kustomize vs Helm):" % component,
        file=sys.stderr,
    )
    for e in errors:
        print("  - " + e, file=sys.stderr)
    sys.exit(1)

print(
    "cross-format parity OK — %s: %d resources field-identical between "
    "Kustomize and Helm." % (component, len(kuz))
)
PY
}

case "$target" in
  webhook)
    check_component "webhook"
    ;;
  equalizer)
    check_component "equalizer"
    ;;
  both)
    check_component "webhook"
    check_component "equalizer"
    ;;
  *)
    echo "usage: $0 [webhook|equalizer|both]" >&2
    exit 2
    ;;
esac
