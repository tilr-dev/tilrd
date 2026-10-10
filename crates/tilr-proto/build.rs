fn main() {
    println!("cargo:rerun-if-changed=proto/tilr.proto");
    prost_build::Config::new()
        .bytes([
            ".tilr.v1.RawInput.data",
            ".tilr.v1.ClipboardPut.data",
            ".tilr.v1.Image.png",
            ".tilr.v1.FsWrite.data",
            ".tilr.v1.FsData.data",
        ])
        .compile_protos(&["proto/tilr.proto"], &["proto"])
        .expect("failed to compile tilr.proto");
}
