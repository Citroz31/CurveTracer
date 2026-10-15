//! Windows: embeds the icon, the version information (Explorer → Properties →
//! Details) and the application manifest in CurveTracer.exe.

fn main() {
    println!("cargo:rerun-if-changed=assets/curvetracer.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/curvetracer.ico")
        .set("ProductName", "CurveTracer")
        .set("FileDescription", "CurveTracer - measurement curve plotting")
        .set("CompanyName", "CurveTracer")
        .set("LegalCopyright", "Copyright (c) 2026 CurveTracer contributors")
        .set("OriginalFilename", "CurveTracer.exe")
        .set("InternalName", "CurveTracer")
        .set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="CurveTracer" version="1.0.0.0"/>
  <description>CurveTracer</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>
"#,
        );
    if let Err(e) = res.compile() {
        panic!("Cannot embed the Windows resources: {e}");
    }
}
