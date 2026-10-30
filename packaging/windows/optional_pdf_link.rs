//! Build-only MSVC policy shared by GUI and CLI entry points.
pub fn configure() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Windows.Data.Pdf is optional: resolve PdfCreateRenderer only after our
        // safe system-directory preflight, never in the Windows startup loader.
        println!("cargo:rustc-link-lib=delayimp");
        println!("cargo:rustc-link-arg=/DELAYLOAD:windows.data.pdf.dll");
        println!("cargo:rustc-link-arg=/DELAYLOAD:prntvpt.dll");
    }
}
