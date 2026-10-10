//! On Windows, `skillpouch.exe` carries the SkillPouch icon and name, so
//! Explorer, Task Manager and the file's Properties show them. Nothing
//! happens for other targets.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/skillpouch.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/skillpouch.ico")
        .set("FileDescription", "SkillPouch")
        .set("ProductName", "SkillPouch");
    if let Err(error) = resource.compile() {
        // Releases are built on Windows, where it must work. Elsewhere (a
        // cross build without a resource compiler) the program still builds.
        if cfg!(windows) {
            panic!("couldn't add the icon to skillpouch.exe: {error}");
        }
        println!("cargo:warning=skillpouch.exe gets no icon: {error}");
    }
}
