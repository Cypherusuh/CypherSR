fn main() {
    let cargo_version = env!("CARGO_PKG_VERSION");

    assert!(
        cargo_version.split('.').count() == 3
            && cargo_version
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())),
        "desktop/Cargo.toml version must use x.y.z format"
    );

    // Desktop version is tracked independently from the backend app version so
    // backend-only updates can ship without forcing a frontend release.
    println!("cargo:rustc-env=CYPHERSR_APP_VERSION={cargo_version}");

    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=ui/main.slint");

    slint_build::compile("ui/main.slint").unwrap();

    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");

        resource
            .compile()
            .expect("Failed to embed the CypherSR Desktop Windows icon");
    }
}
