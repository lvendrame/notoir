# Notoir user guide

The source of the guide published at <https://lvendrame.github.io/notoir/>, built with
[mdBook](https://rust-lang.github.io/mdBook/) (`mdbook serve docs/guide`).

- Chapters are hand-written Markdown in `src/`. Describe what the app does today; the normative behaviour is in
  `docs/specs/`.
- `src/reference/` is generated from the command registry, the settings schema, the menus and the MCP tool catalogue:
  `cargo run -p notoir --example gen_docs`. `cargo test -p notoir --test docs` fails when it is stale, when a link or
  image is missing, when an image is unused, or when a page is missing from `SUMMARY.md`.
- Screenshots in `src/images/` are rendered by `apps/notoir/tests/docs_shots.rs` (and the theme and language images by
  `apps/notoir/tests/render.rs`). `scripts/docs-shots.sh` refreshes all of them and the reference pages.
