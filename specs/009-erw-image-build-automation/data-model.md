# Data Model: ERW Verify Image Build Automation

## §1 — BuildConfig

The resolved configuration for the image build+push phase. Populated from
CLI flags → `.env` → ambient environment → defaults.

| Field | Type | Source | Default |
|-------|------|--------|--------|
| `registry` | `String` | `--registry` → `ERW_REGISTRY` → ambient | (required, no default) |
| `image_name` | `String` | `--image-name` → `ERW_IMAGE_NAME` → ambient | `capacity-admission-webhook` |
| `image_tag` | `String` | `--image-tag` → `ERW_IMAGE_TAG` → ambient | `latest` |
| `kubeconfig` | `Option<PathBuf>` | `--kubeconfig` → `ERW_KUBECONFIG` → `KUBECONFIG` → `Config::infer` | `None` |
| `skip_build` | `bool` | `--skip-build` → `ERW_SKIP_BUILD` → ambient | `false` |
| `json` | `bool` | `--json` | `false` |
| `keep_on_failure` | `bool` | `--keep-on-failure` | `false` |
| `timeout_secs` | `u64` | `--timeout-secs` → `VERIFY_TIMEOUT_SECS` → ambient | `120` |

**Derived field**: `fully_qualified_image` = `"{registry}/{image_name}:{image_tag}"`.

## §2 — EnvFile

A parsed `.env` file — a simple key-value map.

```rust
// Pure function, fully unit-testable.
pub fn parse_env_file(contents: &str) -> BTreeMap<String, String>
```

**Parsing rules**:
- Lines starting with `#` are comments (ignored).
- Empty lines are ignored.
- `KEY=VALUE` format.
- Leading/trailing whitespace around KEY and VALUE is trimmed.
- Values may be wrapped in double quotes (`KEY="value"`) or single quotes
  (`KEY='value'`) — the quotes are stripped.
- A line without `=` is ignored (malformed, not an error).
- Duplicate keys: last wins (standard `.env` semantics).

## §3 — Pipeline state machine

The extended lifecycle (spec-005 lifecycle + build prefix):

```
LoadEnv → (BuildImage → PushImage)? → ConnectClient → PreFlight →
  Setup → Scenarios → Teardown → Report
```

The `?` means the BuildImage→PushImage phase is skipped when `skip_build` is
true. All other phases are unchanged from spec-005.

**Exit codes** (extends spec-005 data-model §3):
- `0` — all scenarios passed + teardown succeeded
- `1` — one or more scenarios failed
- `2` — setup error (including build/push failure — new)
- `3` — teardown partial failure
- `4` — configuration error (.env missing, required variable missing, docker
  not found) — NEW
