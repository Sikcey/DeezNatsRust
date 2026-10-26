fn main() {
    println!("cargo:rerun-if-changed=../../assets/DeezNats.ico");
    println!("cargo:rerun-if-changed=../../assets/app.manifest");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../../assets/DeezNats.ico")
        .set_manifest_file("../../assets/app.manifest")
        .compile()
        .expect("could not embed the Windows icon and control manifest");
}
