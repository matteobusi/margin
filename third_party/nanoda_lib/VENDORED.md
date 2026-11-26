# Vendored nanoda_lib

- Upstream: https://github.com/ammkrn/nanoda_lib
- Commit: `0da9f98feadd3a0a2ef4b3dbcf52a298a6fef929` (version 0.4.32)
- License: Apache-2.0 (see `LICENSE`)

## Local patches

1. `src/parser.rs` (visibility and refactoring only, no logic change):
   - `parse_export_file` is `pub`.
   - The export record types (`ExportJsonObject`, `ExportJsonVal`, `BackRef`, `IndInfo`,
     `Constructor`, `Recursor`, `RecursorRule`, `FileMeta`, ...) and their fields are `pub`.
   - `Parser::go1` is split into JSON deserialization and a new public
     `Parser::add_record`, which holds the original per-line processing unchanged.
   - The end of `parse_export_file` moved into a new public `Parser::finish`.
   This lets the guest feed typed records directly, skipping JSON text, while every
   validation step in the parser still runs. nanoda's own test suite passes.
2. `Cargo.toml`: removed the `nanoda_bin` binary target and the release/test profiles. `src/main.rs` is removed.

No change to the type checker itself. Check with:

```bash
diff -r <upstream>/src third_party/nanoda_lib/src
```
