use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR must be set"));

    let path = which::which("rustc").expect("Couldn't find rustc on path");
    let utf8_path = path.to_str().expect("Path isn't valid UTF-8");
    std::fs::write(out_dir.join("rustc_path"), utf8_path).unwrap();

    let path = which::which("cargo").expect("Couldn't find cargo on path");
    let utf8_path = path.to_str().expect("Path isn't valid UTF-8");
    std::fs::write(out_dir.join("cargo_path"), utf8_path).unwrap();
}
