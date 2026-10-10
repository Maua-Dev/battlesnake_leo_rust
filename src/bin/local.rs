//! Servidor HTTP local, só para testar a cobra na arena (CLI do Battlesnake).
//! Não é usado no deploy: na AWS quem fala HTTP é o `main.rs` via Lambda.
//!
//! Uso:
//!   cargo run --release --bin local            (porta 8080)
//!   PORT=9000 cargo run --release --bin local
//!
//! Usa só a biblioteca padrão para não adicionar dependências ao projeto.

#[path = "../logic.rs"]
mod logic;
#[path = "../models.rs"]
mod models;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).expect("não consegui abrir a porta");
    eprintln!("cobra ouvindo em http://localhost:{port}");
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || handle(stream));
    }
}

fn handle(stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().expect("clone do socket"));
    let mut stream = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("/").to_string();

        let mut content_length = 0usize;
        let mut close = false;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 {
                return;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let lower = h.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            } else if lower.starts_with("connection:") && lower.contains("close") {
                close = true;
            }
        }
        let mut body = vec![0u8; content_length];
        if content_length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }

        let (status, response) = route(&method, &path, &body);
        let msg = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{response}",
            response.len()
        );
        if stream.write_all(msg.as_bytes()).is_err() || close {
            return;
        }
    }
}

fn route(method: &str, path: &str, body: &[u8]) -> (&'static str, String) {
    let path = path.trim_end_matches('/');
    let parse = || serde_json::from_slice::<models::GameState>(body);
    match (method, path) {
        ("POST", p) if p.ends_with("/start") => match parse() {
            Ok(s) => {
                logic::start(&s);
                ("200 OK", "{}".to_string())
            }
            Err(e) => ("400 Bad Request", format!("{{\"error\":\"{e}\"}}")),
        },
        ("POST", p) if p.ends_with("/move") => match parse() {
            Ok(s) => {
                let start = std::time::Instant::now();
                let mv = logic::get_move(&s);
                if std::env::var_os("SNAKE_LOG").is_some() {
                    eprintln!(
                        "turn {} {} {} {}ms",
                        s.turn,
                        mv["move"].as_str().unwrap_or("?"),
                        mv["shout"].as_str().unwrap_or(""),
                        start.elapsed().as_millis()
                    );
                }
                ("200 OK", mv.to_string())
            }
            Err(e) => ("400 Bad Request", format!("{{\"error\":\"{e}\"}}")),
        },
        ("POST", p) if p.ends_with("/end") => match parse() {
            Ok(s) => {
                logic::end(&s);
                ("200 OK", "{}".to_string())
            }
            Err(e) => ("400 Bad Request", format!("{{\"error\":\"{e}\"}}")),
        },
        _ => ("200 OK", logic::info().to_string()),
    }
}
