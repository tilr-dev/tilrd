use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

/// A CDN in a directory: serves the files under `root` on a loopback port.
/// A file that has a twin named `<file>.cut` breaks off halfway. Returns the
/// base URL.
pub fn serve_files(root: PathBuf) -> String {
    serve_files_with_requests(root).0
}

pub fn serve_files_with_requests(
    root: PathBuf,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split(' ').nth(1).unwrap_or("/");
            recorded.lock().unwrap().push(path.to_owned());
            let file = root.join(path.trim_start_matches('/'));
            let mut cut = file.as_os_str().to_owned();
            cut.push(".cut");
            let cut = PathBuf::from(cut).exists();
            let _ = match std::fs::read(file) {
                Ok(body) if cut => stream.write_all(
                    &[
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len())
                            .as_bytes(),
                        &body[..body.len() / 2],
                    ]
                    .concat(),
                ),
                Ok(body) => stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .and_then(|()| stream.write_all(&body)),
                Err(_) => stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                ),
            };
        }
    });
    (format!("http://{addr}"), requests)
}
