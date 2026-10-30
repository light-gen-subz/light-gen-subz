fn main() {
    // whisper.cpp (Metal) and ONNX Runtime (CoreML) call clang's `@available` check,
    // implemented in clang's runtime library. rustc links with `-nodefaultlibs`, so the
    // library has to be named explicitly or the macOS link fails with
    // `Undefined symbols: ___isPlatformVersionAtLeast`.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        link_clang_runtime();
    }
    tauri_build::build()
}

fn link_clang_runtime() {
    let Ok(out) = std::process::Command::new("clang")
        .arg("--print-runtime-dir")
        .output()
    else {
        println!("cargo:warning=clang not found: the clang runtime is not linked");
        return;
    };
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if dir.is_empty() {
        return;
    }
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
