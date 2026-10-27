fn main() {
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // App icon and version info for Explorer and the installer.
        embed_resource::compile("assets/glint.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
