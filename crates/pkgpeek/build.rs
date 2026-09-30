fn main() {
    println!("cargo:rerun-if-changed=../../assets/pkgpeek.rc");
    println!("cargo:rerun-if-changed=../../assets/pkg.ico");
    embed_resource::compile("../../assets/pkgpeek.rc", version_macros())
        .manifest_optional()
        .unwrap();
}

/// Version macros for assets/pkgpeek.rc, from the crate version.
fn version_macros() -> Vec<String> {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts = version.split(['.', '-']).map(|p| p.parse::<u16>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    let (major, minor, patch) = (next(), next(), next());
    vec![
        format!("PKGPEEK_VERSION_MAJOR={major}"),
        format!("PKGPEEK_VERSION_MINOR={minor}"),
        format!("PKGPEEK_VERSION_PATCH={patch}"),
        format!("PKGPEEK_VERSION_STR=\"{version}\""),
    ]
}
