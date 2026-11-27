use super::*;
use perfuse::{
    media::{AudioTrack, Book, Chapter, Kind, Stream, Version},
    plugins::{Credentials, ServerKind, audiobookshelf::PLUGIN},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
};

// Four seconds of unsigned 8-bit mono PCM at 8 kHz.
fn silent_wav() -> Vec<u8> {
    let samples = 32_000_u32;
    let mut wav = b"RIFF".to_vec();
    wav.extend((samples + 36).to_le_bytes());
    wav.extend(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00");
    wav.extend(8_000_u32.to_le_bytes());
    wav.extend(8_000_u32.to_le_bytes());
    wav.extend(b"\x01\x00\x08\x00data");
    wav.extend(samples.to_le_bytes());
    wav.resize(44 + samples as usize, 128);
    wav
}

#[test]
#[ignore = "requires a native libmpv runtime"]
fn native_book_resume_seek_and_finish() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let source = (PLUGIN.open)(
        &Credentials {
            kind: ServerKind::Audiobookshelf,
            server,
            token: "test-token".into(),
            client_id: "perfuse-book-test".into(),
        },
        perfuse::plugins::OpenOptions::default(),
    )
    .unwrap();
    let (serving, shutdown) = mpsc::channel::<()>();
    listener.set_nonblocking(true).unwrap();
    let fixture = thread::spawn(move || {
        let wav = silent_wav();
        let mut tracks = Vec::new();
        let mut positions = Vec::new();
        while shutdown.try_recv() == Err(mpsc::TryRecvError::Empty) {
            let (mut socket, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&socket);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap().to_owned();
            let mut length = 0;
            let mut range = None;
            let mut authenticated = false;
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let (key, value) = line.split_once(':').unwrap();
                if key.eq_ignore_ascii_case("Range") {
                    range = value
                        .trim()
                        .strip_prefix("bytes=")
                        .and_then(|v| v.split_once('-'))
                        .and_then(|(start, _)| start.parse::<usize>().ok());
                } else if key.eq_ignore_ascii_case("Content-Length") {
                    length = value.trim().parse().unwrap();
                } else if key.eq_ignore_ascii_case("Authorization") {
                    authenticated = value.trim() == "Bearer test-token";
                }
            }
            assert!(authenticated, "missing test authorization");
            if path == "/api/me/progress/book" {
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(report["duration"], 8.0);
                positions.push(report["currentTime"].as_f64().unwrap());
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            } else {
                let track = path.strip_prefix("/api/items/book/file/").unwrap();
                assert!(matches!(track, "a" | "b"));
                tracks.push(track.to_owned());
                let start = range.unwrap_or_default();
                assert!(start < wav.len());
                let status = if range.is_some() {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                write!(socket, "HTTP/1.1 {status}\r\nContent-Type: audio/wav\r\nAccept-Ranges: bytes\r\nConnection: close\r\n").unwrap();
                if range.is_some() {
                    write!(
                        socket,
                        "Content-Range: bytes {start}-{}/{}\r\n",
                        wav.len() - 1,
                        wav.len()
                    )
                    .unwrap();
                }
                write!(socket, "Content-Length: {}\r\n\r\n", wav.len() - start).unwrap();
                let _ = socket.write_all(&wav[start..]);
            }
        }
        tracks.dedup();
        (tracks, positions)
    });
    let item = Item {
        id: "book".into(),
        kind: Kind::Audiobook,
        duration_ms: Some(8_000),
        resume_ms: Some(5_000),
        book: Book {
            chapters: (0..4)
                .map(|i| Chapter {
                    title: format!("Chapter {}", i + 1),
                    start_ms: i * 2_000,
                    end_ms: (i + 1) * 2_000,
                })
                .collect(),
            ..Default::default()
        },
        versions: vec![Version {
            stream: Stream::Audiobook(vec![
                AudioTrack {
                    file: "a".into(),
                    start_ms: 0,
                },
                AudioTrack {
                    file: "b".into(),
                    start_ms: 4_000,
                },
            ]),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (controls, receiver) = mpsc::channel();
    let (tx, events) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = play(
            Output::Null,
            source.as_ref(),
            &item,
            StartOptions {
                version: 0,
                resume: true,
                volume: 27.0,
            },
            &receiver,
            &tx,
        );
        tx.send(Event::Finished(result)).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut seek_sent = false;
    let mut heard_first_track = false;
    let mut advanced = false;
    loop {
        match events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Event::Playing {
                seconds,
                duration,
                volume,
                ..
            } => {
                assert_eq!(duration, 8.0);
                if !seek_sent {
                    assert_eq!(
                        volume, 27.0,
                        "initial file should use the remembered volume"
                    );
                    assert!((5.0..7.5).contains(&seconds), "resume position {seconds}");
                    controls.send(Control::Volume(35.0)).unwrap();
                    controls.send(Control::Seek(2.5)).unwrap();
                    seek_sent = true;
                } else if (2.45..4.0).contains(&seconds) {
                    assert_eq!(volume, 35.0);
                    heard_first_track = true;
                } else if heard_first_track && seconds >= 4.0 {
                    assert_eq!(volume, 35.0);
                    advanced = true;
                }
            }
            Event::Warning(message) => panic!("{message}"),
            Event::BookCompleted => break,
            Event::ChapterSleepExpired(_) => panic!("sleep timer was off"),
            Event::Finished(result) => {
                result.unwrap();
                panic!("the completed book should remain available for replay");
            }
        }
    }
    controls.send(Control::Volume(0.0)).unwrap();
    controls.send(Control::Pause).unwrap();
    let mut heard_muted_replay = false;
    loop {
        match events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Event::Playing {
                seconds,
                paused: false,
                volume,
                ..
            } => {
                assert!(seconds < 4.0, "replay should start on the first track");
                if !heard_muted_replay {
                    assert_eq!(volume, 0.0);
                    heard_muted_replay = true;
                    controls.send(Control::Volume(35.0)).unwrap();
                } else if volume == 35.0 && seconds >= 0.5 {
                    break;
                }
            }
            Event::Playing { .. } => {}
            Event::Warning(message) => panic!("{message}"),
            _ => panic!("the completed book should restart when Play is pressed"),
        }
    }
    // Arm in chapter one, seek forward to chapter three, then rearm at six
    // seconds and seek back to chapter two. Its end coincides with a file change.
    for (target, end) in [(5.2, 6.0), (3.3, 4.0)] {
        controls.send(Control::SleepAfterChapter(true)).unwrap();
        controls.send(Control::Seek(target)).unwrap();
        controls.send(Control::SetPaused(false)).unwrap();
        let mut expired = false;
        loop {
            match events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
            {
                Event::ChapterSleepExpired(position) => {
                    assert!(!expired, "the timer must expire once");
                    assert_eq!(position, end);
                    expired = true;
                }
                Event::Playing {
                    seconds,
                    paused: true,
                    ..
                } if expired => {
                    assert!(
                        (seconds - end).abs() < 0.03,
                        "paused at {seconds}, expected {end}"
                    );
                    break;
                }
                Event::Playing { .. } => {}
                Event::Warning(message) => panic!("{message}"),
                _ => panic!("a chapter timer should pause playback at the selected chapter's end"),
            }
        }
    }
    controls.send(Control::Stop).unwrap();
    loop {
        match events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Event::Finished(result) => {
                result.unwrap();
                break;
            }
            Event::Playing { .. } => {}
            Event::Warning(message) => panic!("{message}"),
            Event::BookCompleted => panic!("manual Stop should dismiss the book"),
            Event::ChapterSleepExpired(_) => panic!("the timer expired twice"),
        }
    }
    worker.join().unwrap();
    drop(serving);
    let (tracks, positions) = fixture.join().unwrap();
    assert!(
        advanced,
        "the second track should follow the first on the book timeline"
    );
    assert!(tracks.starts_with(&["b", "a", "b", "a", "b", "a"].map(str::to_owned)));
    assert_eq!(positions.first(), Some(&5.0));
    assert!(
        positions.windows(2).any(|p| p == [8.0, 0.0]),
        "the server should receive the completed position, then the start of replay"
    );
    assert!((positions.last().unwrap() - 4.0).abs() < 0.03);
}
