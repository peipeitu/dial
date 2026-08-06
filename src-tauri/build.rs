fn main() {
    println!("cargo:rerun-if-changed=../src/renderer");
    tauri_build::build();
}
