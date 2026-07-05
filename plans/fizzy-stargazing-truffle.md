## Context

The `edit_files` function in `src/gitops/mod.rs` silently fails when deserializing Kubernetes Deployment YAML into `k8s_openapi::api::apps::v1::Deployment`. The error from `serde_yaml::from_str` is swallowed — only `"Not a deployment {path}"` is logged, giving zero debugging information.

The user's `deployment.yaml` for firefly-iii fails to parse as a Deployment despite being a valid K8s manifest.

## Problem

`serde_yaml` 0.9.x has known issues deserializing `k8s_openapi` types. Specifically, the `Quantity` newtype struct (used for resource requests/limits like `cpu: 6000m` and `memory: 6Gi`) fails because serde_yaml 0.9.x doesn't properly forward `deserialize_newtype_struct` for all value types.

## Fix

1. **Add `Repr` adapter** — Use `serde_yaml::Value` as an intermediate step: parse YAML → `serde_yaml::Value` → convert to JSON → deserialize into `k8s_openapi` types via `serde_json`. This avoids serde_yaml's newtype deserialization bugs entirely.

2. **Log actual errors** — Always log the deserialization error, not just "Not a deployment".

## Files to modify

- `src/gitops/mod.rs` — Rewrite the YAML parsing in `edit_files()` to go through `serde_yaml::Value` → `serde_json::Value` → `k8s_openapi` types. Add error logging.

## Verification

- `cargo build` — compiles
- `cargo clippy` — no warnings
- `cargo test` — tests pass
- Test parsing the actual `deployment.yaml` file manually
