fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-search=native=FlyingCarpetMac/Build/Products/Release");
        println!("cargo:rustc-link-lib=static=FlyingCarpetMac");
    }
}
