fn main() {
    // sqlx::migrate! embeds files; newly added migrations must invalidate incremental builds.
    println!("cargo:rerun-if-changed=../../migrations");
}
