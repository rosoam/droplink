use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
pub enum LogWriter {
    File(Arc<Mutex<std::fs::File>>),
    Stderr,
}

impl LogWriter {
    pub fn open(path: &std::path::Path) -> Self {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => Self::File(Arc::new(Mutex::new(file))),
            Err(_) => Self::Stderr,
        }
    }
}

pub enum LogGuard {
    File(Arc<Mutex<std::fs::File>>),
    Stderr(io::Stderr),
}

impl Write for LogGuard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            LogGuard::File(file) => match file.lock() {
                Ok(mut file) => file.write(buf),
                Err(_) => Ok(buf.len()),
            },
            LogGuard::Stderr(stderr) => stderr.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            LogGuard::File(file) => match file.lock() {
                Ok(mut file) => file.flush(),
                Err(_) => Ok(()),
            },
            LogGuard::Stderr(stderr) => stderr.flush(),
        }
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogGuard;

    fn make_writer(&'a self) -> Self::Writer {
        match self {
            LogWriter::File(file) => LogGuard::File(file.clone()),
            LogWriter::Stderr => LogGuard::Stderr(io::stderr()),
        }
    }
}
