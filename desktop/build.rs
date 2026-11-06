use std::fs;
use std::path::PathBuf;

fn main() {
    let app_version_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app-version.json");
    println!("cargo:rerun-if-changed={}", app_version_path.display());

    let app_version_json =
        fs::read_to_string(&app_version_path).expect("Failed to read repository app-version.json");
    let app_version_config: serde_json::Value = serde_json::from_str(&app_version_json)
        .expect("Failed to parse repository app-version.json");
    let app_version = app_version_config
        .get("version")
        .and_then(serde_json::Value::as_str)
        .expect("app-version.json must contain a string version");

    assert!(
        app_version.split('.').count() == 3
            && app_version
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())),
        "app-version.json version must use x.y.z format"
    );
    println!("cargo:rustc-env=CYPHERSR_APP_VERSION={app_version}");

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
