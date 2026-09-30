fn main() {
    println!("cargo:rerun-if-changed=../../assets/pkgpeek.rc");
    println!("cargo:rerun-if-changed=../../assets/pkg.ico");
    // compile() only links into binaries; the DLL needs every target.
    embed_resource::compile_for_everything("../../assets/pkgpeek.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
