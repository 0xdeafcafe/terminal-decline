// term-app's main.rs without its inner doc comment (include! can't take
// inner attributes), into OUT_DIR.
fn main() {
    let src = "../../../term-app/src/main.rs";
    println!("cargo:rerun-if-changed={src}");
    let text = std::fs::read_to_string(src).unwrap();
    let body: String = text.lines().filter(|l| !l.starts_with("//!")).map(|l| format!("{l}\n")).collect();
    std::fs::write(format!("{}/main.rs", std::env::var("OUT_DIR").unwrap()), body).unwrap();
}
