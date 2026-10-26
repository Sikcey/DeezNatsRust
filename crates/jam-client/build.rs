fn main() {
    println!("cargo:rerun-if-changed=../../assets/DeezNats.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new().set_icon("../../assets/DeezNats.ico").set_manifest_file("../../assets/app.manifest").compile().expect("Could not embed app resources");
}
