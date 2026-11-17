const PROTOS: &[&str] = &[
    "proto/bus.proto",
    "proto/tilr/bus/v1/methods.proto",
    "proto/tilr/pane/v1/frame.proto",
    "proto/tilr/pane/v1/input.proto",
    "proto/tilr/pane/v1/pane.proto",
    "proto/tilr/server/v1/server.proto",
    "proto/tilr/fs/v1/fs.proto",
    "proto/tilr/exec/v1/exec.proto",
    "proto/tilr/clipboard/v1/clipboard.proto",
    "proto/tilr/host/v1/host.proto",
    "proto/tilr/agent/v1/agent.proto",
];

fn main() {
    println!("cargo:rerun-if-changed=proto");
    for proto in PROTOS {
        println!("cargo:rerun-if-changed={proto}");
    }
    prost_build::Config::new()
        // Every `bytes` field is `bytes::Bytes`: payloads pass through without a copy.
        .bytes(["."])
        // A frame is far larger than the other variant of its oneof.
        .boxed(".tilr.pane.v1.PaneOutput.output.frame")
        .compile_protos(PROTOS, &["proto"])
        .expect("failed to compile the bus schemas");
}
