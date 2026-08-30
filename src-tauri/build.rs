fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/macos_ocr.m")
            .file("native/macos_pdf.m")
            .flag("-fobjc-arc")
            .compile("smartcat_macos_ocr");
        println!("cargo:rustc-link-lib=framework=Vision");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=PDFKit");
        println!("cargo:rerun-if-changed=native/macos_ocr.m");
        println!("cargo:rerun-if-changed=native/macos_pdf.m");
    }
    tauri_build::build()
}
