//! Embed the Windows resources (application icon + manifest) into the `rustshot` executable.
//! The manifest declares Per-Monitor-V2 DPI awareness and `asInvoker` privileges.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=resources/rustshot.ico");
        // Optional for debug builds (the exe still runs without an icon/manifest), but a release
        // build without them would ship DPI-unaware and iconless, so it fails instead.
        if let Err(e) = embed_resource::compile("app.rc", embed_resource::NONE).manifest_optional()
        {
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                panic!("could not embed app resources (icon + manifest) in a release build: {e}");
            }
            println!("cargo:warning=could not embed app resources: {e}");
        }
    }
}
