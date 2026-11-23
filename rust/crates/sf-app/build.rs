//! Gives streamflow.exe its icon and version details (what Explorer, the taskbar and the Start menu show).
//! Without this the release binary carries the generic Windows program icon.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../../build/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../../build/icon.ico");
    res.set("ProductName", "StreamFLOW");
    res.set("FileDescription", "StreamFLOW flow cytometry analysis");
    res.set("LegalCopyright", "Apache-2.0");
    // A machine without the Windows resource compiler can still build and run the app (it just has
    // no icon), so this is a warning. The release workflow checks the icon is really there.
    if let Err(e) = res.compile() {
        println!("cargo:warning=no icon embedded in streamflow.exe: {e}");
    }
}
