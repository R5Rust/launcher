fn main() {
    // The launcher's own icon (dark square, white gear+A) so it doesn't look like the game's banner in the taskbar.
    println!("cargo:rerun-if-changed=assets/app-icon.ico");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app-icon.ico");
        // Common Controls v6: rfd's message boxes (uninstall) import TaskDialogIndirect, which only v6 has; without
        // this manifest Windows binds comctl32 v5 and the exe fails to start (STATUS_ENTRYPOINT_NOT_FOUND).
        resource.set_manifest(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
  processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
</assembly>"#,
        );
        if let Err(error) = resource.compile() {
            println!("cargo:warning=icon not embedded: {error}");
        }
    }
}
