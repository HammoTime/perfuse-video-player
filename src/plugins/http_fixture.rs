use reqwest::Url;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

pub(crate) struct Request {
    pub(crate) url: Url,
    pub(crate) headers: String,
    pub(crate) body: Value,
}

impl Request {
    pub(crate) fn query(&self, key: &str) -> Option<String> {
        self.url
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    }
}

pub(crate) type Reply = (&'static str, u16, Vec<u8>);

pub(crate) fn ok(route: &'static str, body: Value) -> Reply {
    (route, 200, serde_json::to_vec(&body).unwrap())
}

// Successful requests and error responses go through reqwest and the actual plugin.
pub(crate) fn fixture(replies: Vec<Reply>) -> (String, thread::JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let base = Url::parse(&origin).unwrap();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for (route, status, body) in replies {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap();
            let url = base.join(parts.next().unwrap()).unwrap();
            assert_eq!(format!("{method} {}", url.path()), route);
            assert!(
                !url.as_str().contains("test-token") && !url.as_str().contains("test-password")
            );
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let length = headers
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.eq_ignore_ascii_case("Content-Length")
                        .then(|| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let payload = if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            requests.push(Request {
                url,
                headers,
                body: payload,
            });
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            )
            .unwrap();
            if status == 302 {
                write!(stream, "Location: http://127.0.0.1:1/other\r\n").unwrap();
            }
            write!(stream, "\r\n").unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (origin, worker)
}
