fn main() {
    // The compiler appends closures to the function table, so the table must
    // not have a maximum size.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-cdylib-link-arg=--growable-table");
    }
}
