# Research — Multi-Selector Node Exclusion

## R1 — `Option<Vec<LabelSelector>>` (not a wrapper struct)

**Decision**: `spec.nodeSelectors: Option<Vec<LabelSelector>>`.

**Rationale**: the simplest representation. `None` = no selectors (default);
`Some(vec![])` = empty list (also no selectors); `Some(vec![sel1, sel2])` = OR
semantics. No custom wrapper struct is needed — `Vec<LabelSelector>` is directly
serializable, schema-able, and ergonomic. The existing `Option<LabelSelector>`
from spec-006 becomes `Option<Vec<LabelSelector>>`.

**Alternatives**: a wrapper struct (`NodeSelectors { selectors: Vec<...> }`)
adds a serialization layer for no benefit. Rejected.

## R2 — OR via `labels_match_any_selector`

**Decision**: add `fn labels_match_any_selector(labels, selectors: &[LabelSelector]) -> bool`
to `node_filter.rs`. Returns `true` if the labels match ANY selector in the slice.

**Rationale**: reuses the existing `labels_match_selector` (per-selector AND)
and wraps it in an OR loop. ~5 lines. The `is_node_counted` function changes
from taking `Option<&LabelSelector>` to `Option<&[LabelSelector]>`.

## R3 — Clean rename, no backward-compat shim

**Decision**: rename `node_selector` → `node_selectors` across all files. No
dual-field period.

**Rationale**: spec-006 was merged minutes ago. No production deployments,
no external consumers. A clean rename is simpler than maintaining two fields.

## R4 — Per-selector validation, skip-invalid

**Decision**: each selector in the list is validated independently. Invalid
selectors are logged and skipped; valid ones still apply.

**Rationale**: an operator might have 3 selectors where one has a typo. The other
two should still work. The `effective_selectors` function filters the list,
logging warnings for each invalid entry.
