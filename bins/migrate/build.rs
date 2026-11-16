fn main() {
    println!("cargo:rerun-if-changed=../../crates/db/migrations");
}
