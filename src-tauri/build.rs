fn main() {
    tauri_build::build();
    // Tauri embeds this dependency in the app manifest. Integration tests
    // that link the native workflow also import Common Controls v6 APIs;
    // Cargo's test executables do not inherit the application's resource.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
}
