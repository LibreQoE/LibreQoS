use std::env;
use std::path::PathBuf;
use std::process::{Command, Output};

fn command_warnings(section: &str, command_result: &std::io::Result<Output>) {
    if command_result.is_err() {
        println!("cargo:warning=[{section}]{:?}", command_result);
    }

    let r = command_result.as_ref().unwrap().stdout.clone();
    if !r.is_empty() {
        println!("cargo:warning=[{section}]{}", String::from_utf8(r).unwrap());
    }

    let r = command_result.as_ref().unwrap().stderr.clone();
    if !r.is_empty() {
        panic!("{}", String::from_utf8(r).unwrap());
    }
}

fn command_warnings_errors_only(section: &str, command_result: &std::io::Result<Output>) {
    if command_result.is_err() {
        println!("cargo:warning=[{section}]{:?}", command_result);
    }

    let r = command_result.as_ref().unwrap().stderr.clone();
    if !r.is_empty() {
        println!("cargo:warning=[{section}] {}", String::from_utf8(r).unwrap());
    }
}

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();

    // 1: Shell out to build the lqos_bpf.ll XDP/TC combined program.
    // Command line to wrap:
    // clang -S -target bpf -Wall -Wno-unused-value -Wno-pointer-sign -Wno-compare-distinct-pointer-types -Werror -emit-llvm -c -g -I../headers/ -O2 -o "../bin"/libre_xdp_kern.ll libre_xdp_kern.c

    let build_target = format!("{}/lqos_kern.ll", out_dir.to_str().unwrap());
    let compile_result = Command::new("clang")
        .current_dir("src/bpf")
        .args([
            "-S",
            "-target",
            "bpf",
            "-Wall",
            "-Wno-unused-value",
            "-Wno-pointer-sign",
            "-Wno-compare-distinct-pointer-types",
            "-Werror",
            "-emit-llvm",
            "-c",
            "-g",
            "-O2",
            "-o",
            &build_target,
            "lqos_kern.c",
        ])
        .output();
    command_warnings("clang", &compile_result);

    // 2: Link the .ll file into a .o file
    // Command line:
    // llc -march=bpf -filetype=obj -o "../bin"/libre_xdp_kern.o "../bin"/libre_xdp_kern.ll
    let link_target = format!("{}/lqos_kern.o", out_dir.to_str().unwrap());
    let link_result = Command::new("llc")
        .args([
            "-march=bpf",
            "-filetype=obj",
            "-o",
            &link_target,
            &build_target,
        ])
        .output();
    command_warnings("llc", &link_result);

    // 3: Use bpftool to build the skeleton file
    // Command line:
    // bpftool gen skeleton ../bin/libre_xdp_kern.o > libre_xdp_skel.h
    let skel_target = format!("{}/lqos_kern_skel.h", out_dir.to_str().unwrap());
    let skel_result = Command::new("bpftool")
        .args(["gen", "skeleton", &link_target])
        .output();
    command_warnings_errors_only("bpf skel", &skel_result);
    let header_file = String::from_utf8(skel_result.unwrap().stdout).unwrap();
    std::fs::write(&skel_target, header_file).unwrap();

    // 4: Copy the wrapper to our out dir
    let wrapper_target = format!("{}/wrapper.h", out_dir.to_str().unwrap());
    let wrapper_target_c = format!("{}/wrapper.c", out_dir.to_str().unwrap());
    let shrinkwrap_lib = format!("{}/libshrinkwrap.o", out_dir.to_str().unwrap());
    let shrinkwrap_a = format!("{}/libshrinkwrap.a", out_dir.to_str().unwrap());
    std::fs::copy("src/bpf/wrapper.h", &wrapper_target).unwrap();
    std::fs::copy("src/bpf/wrapper.c", &wrapper_target_c).unwrap();

    // 5: Build the intermediary library
    let build_result = Command::new("clang")
        .current_dir("src/bpf")
        .args([
            "-c",
            "wrapper.c",
            &format!("-I{}", out_dir.to_str().unwrap()),
            "-o",
            &shrinkwrap_lib,
        ])
        .output();
    command_warnings("clang - wrapper", &build_result);

    let _build_result = Command::new("ar")
        .args([
            "r",
            &shrinkwrap_a,
            &shrinkwrap_lib,
            //"/usr/lib/x86_64-linux-gnu/libbpf.a",
        ])
        .output();
    //command_warnings(&build_result);

    println!(
        "cargo:rustc-link-search=native={}",
        out_dir.to_str().unwrap()
    );
    println!("cargo:rustc-link-lib=static=shrinkwrap");

    // 6: Use bindgen to generate a Rust wrapper
    let bindings = bindgen::Builder::default()
        // The input header we would like to generate
        // bindings for.
        .header(&wrapper_target)
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
