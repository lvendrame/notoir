fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if os != "macos" && os != "linux" {
        return;
    }
    let source = if os == "macos" {
        "native/print.m"
    } else {
        "native/print.c"
    };
    println!("cargo:rerun-if-changed={source}");
    let mut build = cc::Build::new();
    build.file(source).cargo_metadata(false);
    if os == "macos" {
        build.flag("-fobjc-arc").flag("-fblocks");
        build.file("native/associations.m");
        println!("cargo:rerun-if-changed=native/associations.m");
        println!("cargo:rustc-link-lib=framework=CoreServices");
        println!("cargo:rustc-link-lib=framework=UniformTypeIdentifiers");
        println!("cargo:rustc-link-lib=framework=Cocoa");
    } else {
        let gtk = pkg_config::Config::new()
            .probe("gtk+-3.0")
            .expect("GTK 3 development libraries");
        for path in gtk.include_paths {
            build.include(path);
        }
    }
    build.compile("notoir_print");
    println!(
        "cargo:rustc-link-search=native={}",
        std::env::var("OUT_DIR").unwrap()
    );
    // The helper enters through a native constructor, before Rust main. No
    // unsafe Rust or exported C ABI is needed; keep the constructor linked.
    println!("cargo:rustc-link-lib=static:+whole-archive=notoir_print");
}
