fn main() {
    println!("cargo:rerun-if-changed=ui/main.slint");

    // NOTE: the build-script process sees a stale snapshot of ui/main.slint,
    // so the source is injected via include_str!() (read by rustc, which sees
    // the working tree) into OUT_DIR and compiled from there.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let src = include_str!("ui/main.slint");
    std::fs::write(out.join("main.slint"), src).unwrap();
    slint_build::compile(out.join("main.slint")).expect("slint build failed");
}
