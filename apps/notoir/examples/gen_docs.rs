//! Writes the generated reference pages of the user guide
//! (`docs/guide/src/reference/`): shortcuts, commands, settings, MCP tools.
//!
//! ```sh
//! cargo run -p notoir --example gen_docs
//! ```

fn main() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide/src/reference");
    std::fs::create_dir_all(&dir).expect("create reference dir");
    for (name, text) in notoir::docs_ref::render_all() {
        std::fs::write(dir.join(name), text).expect("write reference page");
        println!("wrote docs/guide/src/reference/{name}");
    }
}
