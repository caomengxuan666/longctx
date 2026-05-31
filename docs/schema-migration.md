# Schema Migration

`longctx` uses `schema_version` as the compatibility marker for generated manifests, results, and run snapshots.

Current policy:

- Prefer additive, backward-compatible changes.
- Preserve readers for older flat benchmark layouts when introducing new manifest layouts.
- Keep optional fields optional in JSON and TOML when possible.
- Bump `schema_version` only when a change is incompatible for existing readers.

When a breaking change is unavoidable:

1. Add a new reader that accepts the old and new layouts if possible.
2. Update generator, runner, report, and validator code together.
3. Update docs and tests for both old and new artifacts.
4. Call out the migration path in release notes or a dedicated doc update.

Known compatible layouts:

- Benchmark manifests under `manifests/*.json`.
- Older flat manifests at the benchmark root.
- Result rows in `results.jsonl` with optional newer fields.
- Nested routing audit objects in result rows, as long as their `schema_version` is supported.
- Run snapshots in `run.json` with optional newer fields.
