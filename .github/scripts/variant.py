"""Rewrites Cargo.toml for a size experiment variant (temporary)."""
import re, sys

v = sys.argv[1]
s = open("Cargo.toml").read()
fat = "fat" in v
noacc = "noacc" in v
feats = ["default_fonts", "links", "wayland", "x11"]
if not noacc:
    feats.append("accesskit")
extra = ""
if v.startswith("glow"):
    feats.append("glow")
elif v.startswith("wgpu"):
    feats.append("wgpu_no_default_features")
    win = ["std", "wgsl", "dx12"] + (["gles"] if "gles" in v else [])
    lin = ["std", "wgsl", "vulkan", "gles"]
    mac = ["std", "wgsl", "metal"]
    q = lambda f: ", ".join('"%s"' % x for x in f)
    extra = (
        "\n[target.'cfg(windows)'.dependencies]\nwgpu = { version = \"30\", default-features = false, features = [%s] }\n"
        "\n[target.'cfg(target_os = \"linux\")'.dependencies]\nwgpu = { version = \"30\", default-features = false, features = [%s] }\n"
        "\n[target.'cfg(target_os = \"macos\")'.dependencies]\nwgpu = { version = \"30\", default-features = false, features = [%s] }\n"
    ) % (q(win), q(lin), q(mac))
if v != "base":
    s = s.replace('eframe = "0.36"', 'eframe = { version = "0.36", default-features = false, features = [%s] }' % ", ".join('"%s"' % f for f in feats))
if fat:
    s = s.replace('lto = "thin"', 'lto = "fat"').replace("codegen-units = 4", "codegen-units = 1")
if "small" in v:
    extra_prof = "".join(
        "\n[profile.release.package.%s]\nopt-level = \"s\"\n" % c
        for c in ["naga", "wgpu-core", "wgpu-hal", "wgpu", "wgpu-types", "egui-wgpu", "codespan-reporting", "rfd", "winit", "egui-winit"]
    )
    s += extra_prof
s = s.replace("[profile.release]", extra + "\n[profile.release]")
open("Cargo.toml", "w").write(s)
print(s)
