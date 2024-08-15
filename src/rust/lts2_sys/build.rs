fn main() {
    // Add the -L flag to the compiler to find the liblts2_client.a in the crate directory
    println!("cargo:rustc-link-search=all={}", env!("CARGO_MANIFEST_DIR"));

    // Statically link liblts2_client.a
    println!("cargo:rustc-link-lib=static=lts2_client");
}