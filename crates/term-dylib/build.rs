// The library's install name (docs/design/pins.md §2.5): the launcher and
// pins load it by path, and nothing records where it was built.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libterminal_decline.dylib");
    }
}
