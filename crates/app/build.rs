//! Embed the Windows resources (application icon + manifest) into the `rustshot` executable.
//! The manifest declares Per-Monitor-V2 DPI awareness and `asInvoker` privileges.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=resources/rustshot.ico");
        // Resources are optional: if rc.exe is unavailable the exe still builds (no icon/manifest).
        if let Err(e) = embed_resource::compile("app.rc", embed_resource::NONE).manifest_optional()
        {
            println!("cargo:warning=could not embed app resources: {e}");
        }
    }
}
