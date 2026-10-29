
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();

        resource.set_icon("assets/icon.ico");

        resource
            .compile()
            .expect("Failed to embed the CypherSR Windows icon");
    }
}
