fn main() {
    tauri_build::build();
    embed_manifest_for_windows_tests();
}

/// Embed the Windows application manifest into test binaries so they
/// declare a dependency on Common Controls v6 — same manifest the
/// production binary already gets via `tauri_build::build()`, but
/// scoped to `cargo test` artifacts where Tauri's normal embed path
/// doesn't run.
///
/// Without this, `cargo test --workspace` on a Windows MSVC runner
/// fails with `STATUS_ENTRYPOINT_NOT_FOUND` (`0xc0000139`) before any
/// test runs: the Windows loader resolves a comctl32 v6 symbol the
/// Tauri runtime imports, picks up the v5 comctl32 instead (no v6
/// manifest → no v6 activation), and rejects the test binary at
/// load time. Mirrors `embed_manifest_for_tests` in Tauri's own
/// `crates/tauri/build.rs` (gated on `__TAURI_WORKSPACE__=true`
/// upstream; we apply it unconditionally here for our test artifacts).
///
/// The `-tests` suffix on the link-arg directives scopes the linker
/// flags to test artifacts only, so the production binary's manifest
/// embedded by `tauri_build::build()` above is not double-applied
/// (which would surface as a "duplicate resource" linker error —
/// Tauri issue #10154).
fn embed_manifest_for_windows_tests() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-tests=/WX");
}
