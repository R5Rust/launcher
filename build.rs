fn main() {
    // The launcher's own icon (dark square, white gear+A) so it doesn't look like the game's banner in the taskbar.
    println!("cargo:rerun-if-changed=assets/app-icon.ico");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app-icon.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=icon not embedded: {error}");
        }
    }
}
