use std::io::BufReader;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

use codegraph_rs::mcp::McpServer;

pub struct McpHandle {
    pub port: u16,
    shutdown: Arc<AtomicBool>,
}

impl McpHandle {
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Connect to self to unblock the blocking accept() call
        let _ = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.port));
    }
}

pub fn start(project_root: PathBuf, port: u16) -> std::io::Result<McpHandle> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;
    let actual_port = listener.local_addr()?.port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    thread::spawn(move || {
        for stream in listener.incoming() {
            if shutdown_clone.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let path = project_root.clone();
            thread::spawn(move || {
                let writer = stream.try_clone().expect("tcp clone");
                let reader = BufReader::new(stream);
                let mut server = McpServer::new(Some(path));
                let _ = server.start_with_io(reader, writer);
            });
        }
    });

    Ok(McpHandle { port: actual_port, shutdown })
}
