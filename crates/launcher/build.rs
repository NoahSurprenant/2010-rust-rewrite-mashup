fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // A release build that updates itself through iw4launcher (src/main.rs).
    println!("cargo::rerun-if-env-changed=IW4L_DEFAULT_UPDATE_URL");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-link-arg-bins=/STACK:8388608");
    }
}
