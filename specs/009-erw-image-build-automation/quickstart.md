# Quickstart: ERW Verify Image Build Automation

## Prerequisites

- Docker installed and on PATH
- Authenticated to a container registry (e.g. `docker login cr.yandex`)
- A clean, throwaway Kubernetes cluster
- A kubeconfig for that cluster

## Setup

1. Copy the `.env.example` to `.env`:
   ```bash
   cp .env.example .env
   ```

2. Fill in your values:
   ```env
   ERW_REGISTRY=cr.yandex/crppbh5k4v76t4ml9u8f
   ERW_IMAGE_NAME=capacity-admission-webhook
   ERW_IMAGE_TAG=latest
   ERW_KUBECONFIG=test.kubeconfig.yaml
   ```

3. Build the verify binary:
   ```bash
   cargo build --bin erw-verify
   ```

## Run the full pipeline

```bash
./target/debug/erw-verify
```

The tool will:
1. Read `.env` from the repo root
2. Build the Docker image (`docker build -t <registry>/<image>:<tag> .`)
3. Push it (`docker push <registry>/<image>:<tag>`)
4. Connect to the cluster via the kubeconfig
5. Install the webhook stack (with the resolved image reference)
6. Run all verification scenarios
7. Tear down everything
8. Print the report

## Skip build (reuse existing image)

```bash
./target/debug/erw-verify --skip-build
```

## Expected output

The report shows all enforcement and degradation scenarios with pass/fail
status, plus the cluster URL, image reference, and total duration.
