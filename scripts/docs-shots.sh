#!/usr/bin/env bash
# Refreshes the user guide's screenshots and generated reference pages
# (docs/guide/README.md). Needs a GPU adapter for the renders.
set -euo pipefail
cd "$(dirname "$0")/.."

images=docs/guide/src/images
shots="$PWD/target/docs-shots"
snapshots=target/ui-snapshots

rm -rf "$shots"
NOTOIR_DOCS_SHOTS="$shots" cargo test -p notoir --test docs_shots
cargo test -p notoir --test render -- syntax_snapshot empty_state_snapshots

# Only the images the guide uses; the scenes render a few extra.
for f in "$images"/*.png; do
  cp "$shots/$(basename "$f")" "$f"
done
for theme in notoir-midnight notoir-paper dracula; do
  cp "$snapshots/syntax-$theme.png" "$images/themes/"
done
for lang in ja de; do
  cp "$snapshots/shell-notoir-paper-$lang.png" "$images/languages/"
done

cargo run -p notoir --example gen_docs
echo "Updated $images and docs/guide/src/reference."
