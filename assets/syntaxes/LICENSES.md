# Syntax definitions

Notoir highlights with [syntect](https://github.com/trishume/syntect). Grammars come from:

| Source | Grammars | License |
|---|---|---|
| [two-face](https://codeberg.org/CosmicHarper/two-face) (the syntax set bundled with [bat](https://github.com/sharkdp/bat)) | syntect defaults plus TOML, TypeScript/TSX, Dockerfile, dotenv, log and many more | MIT OR Apache-2.0 for the crate; each grammar keeps its upstream license, listed in two-face's `acknowledgement` module |
| `PowerShell.sublime-syntax` (this folder) from [SublimeText/PowerShell](https://github.com/SublimeText/PowerShell) | PowerShell | MIT — see `PowerShell.LICENSE.txt` |

Files in this folder are compiled together with the two-face set into one packdump by
`crates/notoir-editor/build.rs`, so nothing is parsed at startup. To add a grammar, drop a `.sublime-syntax` file here
with its license and add a row above.
