use std::fs;
use std::io;
use std::io::BufReader;
use std::io::{Read, Write, BufRead};
use std::net::TcpStream;
use std::path::PathBuf;
use std::thread;

use base64::prelude::*;
use structopt::StructOpt;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(StructOpt, Debug)]
#[structopt(name = "decork")]
struct Opt {
    /// The destination.
    dest: String,

    /// Http Proxy. Default to $http_proxy.
    #[structopt(long)]
    proxy: Option<String>,

    /// Path to the auth file.
    #[structopt(long)]
    auth: Option<PathBuf>,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let default = match std::env::var("http_proxy") {
        Ok(env) => Some(Url::parse(&env)?),
        Err(_) => None,
    };
    let proxy = if let Some(proxy) = opt.proxy {
        proxy.clone()
    } else if let Some(ref default) = default {
        format!(
            "{}:{}",
            default.host_str().expect("hostname"),
            default.port_or_known_default().unwrap_or(8080)
        )
    } else {
        // No proxy.
        return direct_main(&opt.dest);
    };
    let auth = if let Some(auth) = opt.auth {
        Some(fs::read_to_string(auth)?)
    } else if let Some(ref default) = default {
        if let Some(password) = default.password() {
            Some(format!("{}:{}", default.username(), password))
        } else {
            Some(default.username().to_string())
        }
    } else {
        None
    };
    tunnel_main(&proxy, &opt.dest, auth.as_deref())
}

fn direct_main(dest: &str) -> Result<()> {
    let stream = TcpStream::connect(dest)?;
    let stream_in = stream.try_clone()?;
    let stream_out = stream;
    let to_stdout = std::thread::spawn(move || {
        immediate_copy(stream_out, io::stdout()).ok();
    });
    let from_stdin = std::thread::spawn(move || {
        immediate_copy(io::stdin(), stream_in).ok();
    });
    to_stdout.join().unwrap();
    from_stdin.join().unwrap();
    Ok(())
}

fn tunnel_main(proxy: &str, dest: &str, auth: Option<&str>) -> Result<()> {
    let proxy = establish_proxy_tunnel(proxy, dest, auth)?;
    let proxy_in = proxy.try_clone()?;
    let proxy_out = proxy;
    let proxy_stdout = thread::spawn(move || {
        immediate_copy(proxy_out, io::stdout()).ok();
    });
    let stdin_proxy = thread::spawn(move || {
        immediate_copy(io::stdin(), proxy_in).ok();
    });
    proxy_stdout.join().unwrap();
    stdin_proxy.join().unwrap();
    Ok(())
}

fn establish_proxy_tunnel(
    proxy: &str,
    dest: &str,
    auth: Option<&str>,
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(proxy)?;
    let mut request = format!("CONNECT {} HTTP/1.0\r\n", dest);
    if let Some(auth) = auth {
        let auth_base64 = BASE64_STANDARD.encode(auth);
        request.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth_base64));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.contains("200") {
        return Err(format!("Proxy failed: {}", line).into());
    }
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    // handle data that are read but not consumed
    io::stdout().write_all(reader.buffer())?;
    io::stdout().flush()?;
    let stream = reader.into_inner();
    Ok(stream)
}

fn immediate_copy<R, W>(mut input: R, mut output: W) -> io::Result<()>
where
    R: Read,
    W: Write,
{
    let mut buf = [0u8; 8192];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        output.write_all(&buf[..n])?;
        output.flush()?;
    }
}
