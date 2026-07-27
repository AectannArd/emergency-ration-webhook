//! Node filter (spec-006) — the pure decision logic that determines which nodes
//! count toward the capacity aggregate.
//!
//! Two independent exclusion layers, evaluated in order:
//! 1. **Default (unschedulable)**: a node with `spec.unschedulable = true` is
//!    never counted (FR-001). This cannot be disabled — it fixes the
//!    phantom-capacity bug where cordoned/control-plane nodes inflated the pool.
//! 2. **Selector**: a node matching the optional `ClusterCapacity.spec.nodeSelector`
//!    is not counted (FR-003). An absent or empty selector matches nothing, so
//!    only unschedulable nodes are excluded (FR-005).
//!
//! A node counted toward capacity must pass *both* layers (FR-004). The module is
//! pure — no I/O, no client, no async — so every branch is unit-testable in
//! isolation (Constitution Principle VIII).

use std::collections::BTreeMap;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use thiserror::Error;

/// Structural validation error for a `LabelSelector` (spec-006 FR-010). An
/// invalid selector triggers the unschedulable-only fallback (the safe default),
/// never a crash and never a silent partial match.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SelectorError {
    /// `matchExpressions` entry used an operator outside
    /// `{In, NotIn, Exists, DoesNotExist}`. Carries `(operator, key)`.
    #[error("unknown operator '{0}' in matchExpression for key '{1}'")]
    UnknownOperator(String, String),
    /// An `In`/`NotIn` entry has no (or empty) `values`.
    #[error("operator '{operator}' requires non-empty values for key '{key}'")]
    MissingValues { operator: String, key: String },
    /// An `Exists`/`DoesNotExist` entry carries `values` (it must be empty).
    #[error("operator '{operator}' must have empty values for key '{key}'")]
    UnexpectedValues { operator: String, key: String },
}

/// Structurally validate a `LabelSelector` (research R4): every
/// `matchExpressions` entry must use a known operator and satisfy its
/// value-presence rule. `matchLabels` is always structurally valid (plain
/// key→value pairs). Called before label matching; on `Err` the controller falls
/// back to unschedulable-only exclusion for that cycle (FR-010).
pub fn validate_selector(selector: &LabelSelector) -> Result<(), SelectorError> {
    let Some(expressions) = selector.match_expressions.as_ref() else {
        return Ok(());
    };
    for req in expressions {
        let has_values = req.values.as_ref().is_some_and(|values| !values.is_empty());
        match req.operator.as_str() {
            "In" | "NotIn" => {
                if !has_values {
                    return Err(SelectorError::MissingValues {
                        operator: req.operator.clone(),
                        key: req.key.clone(),
                    });
                }
            }
            "Exists" | "DoesNotExist" => {
                if has_values {
                    return Err(SelectorError::UnexpectedValues {
                        operator: req.operator.clone(),
                        key: req.key.clone(),
                    });
                }
            }
            _ => {
                return Err(SelectorError::UnknownOperator(
                    req.operator.clone(),
                    req.key.clone(),
                ));
            }
        }
    }
    Ok(())
}

/// Breakdown of how the node filter disposed of every node in a reconciliation
/// pass. Returned by [`sum_node_allocatable`](super::node_capacity::sum_node_allocatable)
/// so the controller can populate the `ClusterCapacity` status observability
/// fields (spec-006 US3).
///
/// `excluded_node_count()` is always `excluded_unschedulable + excluded_by_selector`:
/// a node that is both unschedulable and selector-matched is counted under
/// `excluded_unschedulable` only (unschedulable is checked first), never
/// double-counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExclusionBreakdown {
    /// Nodes summed toward the capacity aggregate.
    pub counted: i32,
    /// Nodes excluded because `spec.unschedulable = true` (layer 1).
    pub excluded_unschedulable: i32,
    /// Nodes excluded because they matched `spec.nodeSelector` (layer 2).
    pub excluded_by_selector: i32,
}

impl ExclusionBreakdown {
    /// Total nodes excluded from the aggregate — written to status as
    /// `excludedNodeCount`.
    pub fn excluded_node_count(&self) -> i32 {
        self.excluded_unschedulable + self.excluded_by_selector
    }
}

/// Evaluate standard Kubernetes `LabelSelector` semantics (research R2) against
/// a node's labels.
///
/// - An **empty selector** (`matchLabels` and `matchExpressions` both absent or
///   empty) matches **all** nodes → `true` (FR-005, the Kubernetes wildcard).
/// - `matchLabels`: every `{key, value}` must be present in `labels`.
/// - `matchExpressions`: each requirement is evaluated by operator
///   (`In`/`NotIn`/`Exists`/`DoesNotExist`) and the results are ANDed.
///
/// `matchLabels` and `matchExpressions` are ANDed together. This function does
/// not validate the selector — call [`validate_selector`] first; it is only
/// reached for structurally valid selectors (the controller falls back to
/// unschedulable-only exclusion on an invalid one).
fn labels_match_selector(labels: &BTreeMap<String, String>, selector: &LabelSelector) -> bool {
    // matchLabels: every {key, value} must be present exactly.
    if let Some(match_labels) = selector.match_labels.as_ref() {
        for (key, value) in match_labels {
            if labels.get(key) != Some(value) {
                return false;
            }
        }
    }
    // matchExpressions: each requirement must hold (ANDed).
    if let Some(expressions) = selector.match_expressions.as_ref() {
        for req in expressions {
            let node_value = labels.get(&req.key);
            let in_values = |values: &Vec<String>| values.iter().any(|v| node_value == Some(v));
            let matches = match req.operator.as_str() {
                "In" => node_value.is_some_and(|v| {
                    req.values
                        .as_ref()
                        .is_some_and(|values| values.iter().any(|x| x == v))
                }),
                "NotIn" => match req.values.as_ref() {
                    // NotIn matches when the value is absent from the list OR the
                    // key is missing entirely (Kubernetes convention).
                    Some(values) => !in_values(values),
                    None => true,
                },
                "Exists" => node_value.is_some(),
                "DoesNotExist" => node_value.is_none(),
                // Unknown operators never reach here: the controller only calls
                // this after validate_selector succeeds. Treat as non-matching
                // defensively.
                _ => false,
            };
            if !matches {
                return false;
            }
        }
    }
    true
}

/// `true` iff the selector has no `matchLabels` and no `matchExpressions` — the
/// Kubernetes "matches all" wildcard. FR-005: such a selector excludes nothing.
fn selector_is_empty(selector: &LabelSelector) -> bool {
    let no_labels = selector
        .match_labels
        .as_ref()
        .is_none_or(BTreeMap::is_empty);
    let no_exprs = selector
        .match_expressions
        .as_ref()
        .is_none_or(Vec::is_empty);
    no_labels && no_exprs
}

/// The core predicate: should `node` count toward the capacity aggregate?
///
/// - `unschedulable` — `node.spec.unschedulable.unwrap_or(false)`.
/// - `labels` — `node.metadata.labels`.
/// - `selector` — the optional `ClusterCapacity.spec.nodeSelector`.
///
/// Returns `false` if the node is unschedulable (FR-001) or matches a non-empty
/// valid selector (FR-003); `true` otherwise. A `None`/empty selector disables
/// layer 2 (FR-005). A structurally invalid selector is ignored (defensive — the
/// controller pre-validates and falls back to unschedulable-only, FR-010).
pub fn is_node_counted(
    unschedulable: bool,
    labels: Option<&BTreeMap<String, String>>,
    selector: Option<&LabelSelector>,
) -> bool {
    // FR-001: unschedulable nodes are always excluded (the default, cannot disable).
    if unschedulable {
        return false;
    }
    // FR-005: no selector configured → counted (unschedulable-only exclusion).
    let Some(sel) = selector else {
        return true;
    };
    // FR-005: an empty selector matches all nodes → excludes nothing → counted.
    if selector_is_empty(sel) {
        return true;
    }
    // Defensive: never apply a structurally-invalid selector. The controller
    // validates once per cycle and passes None on error, so this is a backstop.
    if validate_selector(sel).is_err() {
        return true;
    }
    // A node with no labels cannot match a non-empty selector → counted.
    let Some(node_labels) = labels else {
        return true;
    };
    // FR-003: a node matching the selector is excluded.
    !labels_match_selector(node_labels, sel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unschedulable_node_is_not_counted() {
        // FR-001: a cordoned node (spec.unschedulable = true) is always excluded,
        // regardless of selector or labels.
        assert!(!is_node_counted(true, None, None));
    }

    #[test]
    fn schedulable_node_with_no_selector_is_counted() {
        // FR-002: a schedulable node with no selector configured is counted.
        assert!(is_node_counted(false, None, None));
    }

    #[test]
    fn schedulable_node_excluded_by_matching_selector() {
        // FR-003 / T024: a schedulable node whose labels match the selector is
        // excluded from the aggregate (e.g. a control-plane node by role label).
        let labels = labels_of(&[("node-role.kubernetes.io/control-plane", "")]);
        let sel = selector(vec![expr(
            "node-role.kubernetes.io/control-plane",
            "Exists",
            None,
        )]);
        assert!(!is_node_counted(false, Some(&labels), Some(&sel)));
    }

    #[test]
    fn schedulable_node_counted_when_selector_does_not_match() {
        // T024: a schedulable node the selector does not match is still counted
        // (passes both exclusion layers — FR-004).
        let labels = labels_of(&[("role", "worker")]);
        let sel = selector(vec![expr("role", "In", Some(&["control-plane"]))]);
        assert!(is_node_counted(false, Some(&labels), Some(&sel)));
    }

    #[test]
    fn schedulable_node_counted_under_empty_selector() {
        // FR-005: an empty selector excludes nothing (no label-based exclusion);
        // the node is counted after passing the unschedulable check.
        let labels = labels_of(&[("role", "worker")]);
        let empty = LabelSelector {
            match_labels: None,
            match_expressions: None,
        };
        assert!(is_node_counted(false, Some(&labels), Some(&empty)));
    }

    // ---- spec-006 US2: validate_selector (T025) ----

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement};

    fn expr(key: &str, op: &str, values: Option<&[&str]>) -> LabelSelectorRequirement {
        LabelSelectorRequirement {
            key: key.to_string(),
            operator: op.to_string(),
            values: values.map(|v| v.iter().map(|s| (*s).to_string()).collect()),
        }
    }

    fn selector(expressions: Vec<LabelSelectorRequirement>) -> LabelSelector {
        LabelSelector {
            match_labels: None,
            match_expressions: Some(expressions),
        }
    }

    #[test]
    fn validate_selector_accepts_valid_operators() {
        // In/NotIn with non-empty values; Exists/DoesNotExist without values.
        assert!(
            validate_selector(&selector(vec![
                expr("role", "In", Some(&["control-plane"])),
                expr("tier", "NotIn", Some(&["edge"])),
                expr("zone", "Exists", None),
                expr("legacy", "DoesNotExist", None),
            ]))
            .is_ok()
        );
        // An empty selector is structurally valid (it matches all nodes).
        assert!(
            validate_selector(&LabelSelector {
                match_labels: None,
                match_expressions: None
            })
            .is_ok()
        );
    }

    #[test]
    fn validate_selector_rejects_unknown_operator() {
        // FR-010: an operator outside {In,NotIn,Exists,DoesNotExist} is invalid.
        assert_eq!(
            validate_selector(&selector(vec![expr("role", "Matches", None)])),
            Err(SelectorError::UnknownOperator(
                "Matches".to_string(),
                "role".to_string()
            ))
        );
    }

    #[test]
    fn validate_selector_rejects_in_without_values() {
        // In (and NotIn) require non-empty values — absent or empty both fail.
        assert_eq!(
            validate_selector(&selector(vec![expr("role", "In", None)])),
            Err(SelectorError::MissingValues {
                operator: "In".to_string(),
                key: "role".to_string()
            })
        );
        assert_eq!(
            validate_selector(&selector(vec![expr("role", "In", Some(&[]))])),
            Err(SelectorError::MissingValues {
                operator: "In".to_string(),
                key: "role".to_string()
            })
        );
    }

    #[test]
    fn validate_selector_rejects_exists_with_values() {
        // Exists/DoesNotExist must have empty/absent values.
        assert_eq!(
            validate_selector(&selector(vec![expr("role", "Exists", Some(&["x"]))])),
            Err(SelectorError::UnexpectedValues {
                operator: "Exists".to_string(),
                key: "role".to_string()
            })
        );
    }

    // ---- spec-006 US2: labels_match_selector (T020-T023) ----

    fn labels_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn match_labels_of(pairs: &[(&str, &str)]) -> LabelSelector {
        LabelSelector {
            match_labels: Some(labels_of(pairs)),
            match_expressions: None,
        }
    }

    #[test]
    fn match_labels_all_present_matches() {
        // T020: every {key,value} in matchLabels must be present in the node's
        // labels. All present → match.
        let labels = labels_of(&[("role", "worker"), ("zone", "a")]);
        assert!(labels_match_selector(
            &labels,
            &match_labels_of(&[("role", "worker")])
        ));
    }

    #[test]
    fn match_labels_value_mismatch_does_not_match() {
        // T020: key present but value differs → no match.
        let labels = labels_of(&[("role", "worker")]);
        assert!(!labels_match_selector(
            &labels,
            &match_labels_of(&[("role", "control-plane")])
        ));
    }

    #[test]
    fn match_labels_missing_key_does_not_match() {
        // T020: a key absent from the node's labels → no match.
        let labels = labels_of(&[("role", "worker")]);
        assert!(!labels_match_selector(
            &labels,
            &match_labels_of(&[("tier", "system")])
        ));
    }

    #[test]
    fn in_operator_matches_only_when_value_listed() {
        // T021: node's value for the key must be in `values`.
        assert!(labels_match_selector(
            &labels_of(&[("zone", "a")]),
            &selector(vec![expr("zone", "In", Some(&["a", "b"]))])
        ));
        assert!(!labels_match_selector(
            &labels_of(&[("zone", "a")]),
            &selector(vec![expr("zone", "In", Some(&["b", "c"]))])
        ));
        // Key absent → In does not match (no value to test membership).
        assert!(!labels_match_selector(
            &BTreeMap::new(),
            &selector(vec![expr("zone", "In", Some(&["a"]))])
        ));
    }

    #[test]
    fn notin_operator_matches_when_value_absent_or_key_missing() {
        // T022: node's value must NOT be in `values` (Kubernetes: NotIn matches
        // when the key is absent too).
        assert!(labels_match_selector(
            &labels_of(&[("zone", "a")]),
            &selector(vec![expr("zone", "NotIn", Some(&["b", "c"]))])
        ));
        assert!(!labels_match_selector(
            &labels_of(&[("zone", "a")]),
            &selector(vec![expr("zone", "NotIn", Some(&["a", "b"]))])
        ));
        assert!(labels_match_selector(
            &BTreeMap::new(),
            &selector(vec![expr("zone", "NotIn", Some(&["a"]))])
        ));
    }

    #[test]
    fn exists_operator_requires_key_present() {
        // T022: Exists matches iff the node has the key (value irrelevant).
        assert!(labels_match_selector(
            &labels_of(&[("role", "anything")]),
            &selector(vec![expr("role", "Exists", None)])
        ));
        assert!(!labels_match_selector(
            &BTreeMap::new(),
            &selector(vec![expr("role", "Exists", None)])
        ));
    }

    #[test]
    fn does_not_exist_operator_requires_key_absent() {
        // T022: DoesNotExist matches iff the node lacks the key.
        assert!(!labels_match_selector(
            &labels_of(&[("role", "worker")]),
            &selector(vec![expr("role", "DoesNotExist", None)])
        ));
        assert!(labels_match_selector(
            &BTreeMap::new(),
            &selector(vec![expr("role", "DoesNotExist", None)])
        ));
    }

    #[test]
    fn empty_selector_matches_all_nodes() {
        // T023 / FR-005: an empty selector (no matchLabels, no matchExpressions)
        // matches every node — the Kubernetes wildcard convention.
        let empty = LabelSelector {
            match_labels: None,
            match_expressions: None,
        };
        assert!(labels_match_selector(&BTreeMap::new(), &empty));
        assert!(labels_match_selector(&labels_of(&[("x", "y")]), &empty));
    }

    #[test]
    fn match_labels_and_expressions_are_anded() {
        // matchLabels and matchExpressions are ANDed: all requirements must hold.
        let labels = labels_of(&[("role", "worker"), ("zone", "a")]);
        let sel = LabelSelector {
            match_labels: Some(labels_of(&[("role", "worker")])),
            match_expressions: Some(vec![expr("zone", "In", Some(&["a", "b"]))]),
        };
        assert!(labels_match_selector(&labels, &sel));
        // If the expression side fails, the whole selector fails.
        let sel_fail = LabelSelector {
            match_labels: Some(labels_of(&[("role", "worker")])),
            match_expressions: Some(vec![expr("zone", "In", Some(&["z"]))]),
        };
        assert!(!labels_match_selector(&labels, &sel_fail));
    }
}
