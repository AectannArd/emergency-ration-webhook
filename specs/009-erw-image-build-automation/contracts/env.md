# Contract: `.env` Variables (spec-009)

## Variable Reference

| Variable | Required | CLI Override | Default | Description |
|----------|----------|-------------|---------|-------------|
| `ERW_REGISTRY` | Yes (unless `--skip-build`) | `--registry` | none | Registry endpoint without protocol (e.g. `cr.yandex/crppbh5k4v76t4ml9u8f`) |
| `ERW_IMAGE_NAME` | No | `--image-name` | `capacity-admission-webhook` | Image name within the registry |
| `ERW_IMAGE_TAG` | No | `--image-tag` | `latest` | Image tag |
| `ERW_KUBECONFIG` | No | `--kubeconfig` | `Config::infer` | Path to kubeconfig (relative to repo root or absolute) |
| `ERW_SKIP_BUILD` | No | `--skip-build` | `false` | Set to `1` or `true` to skip build+push |

## Precedence

1. CLI flag (highest)
2. `.env` file
3. Ambient environment variable
4. Compiled default (lowest)

## Fully-Qualified Image

The resolved image reference is: `{ERW_REGISTRY}/{ERW_IMAGE_NAME}:{ERW_IMAGE_TAG}`

Example: `cr.yandex/crppbh5k4v76t4ml9u8f/capacity-admission-webhook:latest`

## Error Conditions

- `.env` missing AND no CLI flags AND no ambient vars for required fields →
  exit code 4, message: "Missing required configuration: ERW_REGISTRY. Copy
  .env.example to .env and fill in your values."
- `docker` not on `PATH` (and not `--skip-build`) → exit code 4, message:
  "docker not found on PATH. Install Docker or use --skip-build."
