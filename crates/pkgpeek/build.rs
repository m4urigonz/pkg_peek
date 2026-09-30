fn main() {
    println!("cargo:rerun-if-changed=../../assets/pkgpeek.rc");
    println!("cargo:rerun-if-changed=../../assets/pkg.ico");
    embed_resource::compile("../../assets/pkgpeek.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
