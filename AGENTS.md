# AGENTS.md

This repository contains `longctx`, a long-context benchmark CLI.

Working rules for future edits:

- Keep benchmark data reproducible. If generation changes, preserve or extend the `seed` flow.
- Preserve backwards compatibility when possible. The runner should keep reading older flat layouts and old JSON artifacts.
- Treat `schema_version` as the compatibility marker for generated manifests and results.
- Keep benchmark outputs under `bench/contexts`, `bench/manifests`, `bench/results.jsonl`, and `bench/reports`.
- Keep `context = "auto"` routing zero-token and auditable unless a future change explicitly introduces an opt-in LLM router.
- Do not use `expected` answers or answer-like metadata as router input.
- Prefer small, focused changes over wide refactors.
- Update docs whenever CLI flags, config fields, output layout, or result schema change.
- Add tests for generator, grader, runner config parsing, and report generation when touching those areas.
- Use `cargo fmt` and `cargo test` before finishing.
- Use `apply_patch` for file edits. Do not revert user-authored changes.

Useful commands:

```sh
cargo fmt
cargo test
cargo build
```
