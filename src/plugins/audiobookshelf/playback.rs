use super::{Audiobookshelf, identifier};
use crate::media::{AudioTrack, Item, Playback, Progress, SourceResult, Stream, Version};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
pub(super) struct Track {
    ino: String,
    duration: f64,
    #[serde(default)]
    codec: Option<String>,
}

pub(super) fn version(tracks: Vec<Track>) -> SourceResult<(Version, u64)> {
    let audio_codec = tracks.first().and_then(|t| t.codec.clone());
    let mut files = Vec::with_capacity(tracks.len());
    let mut seconds = 0.0_f64;
    for track in tracks {
        identifier(&track.ino)?;
        let next = seconds + track.duration;
        let start_ms = (seconds * 1000.0).round() as u64;
        if !next.is_finite()
            || next * 1000.0 >= u64::MAX as f64
            || (next * 1000.0).round() <= start_ms as f64
        {
            return Err("Audiobookshelf returned an invalid track duration.".into());
        }
        files.push(AudioTrack {
            file: track.ino,
            start_ms,
        });
        seconds = next;
    }
    let version = Version {
        audio_codec,
        stream: if files.is_empty() {
            Stream::Unavailable
        } else {
            Stream::Audiobook(files)
        },
        ..Version::default()
    };
    Ok((version, (seconds * 1000.0).round() as u64))
}

impl Audiobookshelf {
    pub(super) fn book_playback(
        &self,
        item: &Item,
        version: &Version,
        track: usize,
    ) -> SourceResult<Playback> {
        let Stream::Audiobook(tracks) = &version.stream else {
            return Err(version
                .stream
                .direct_file()
                .err()
                .unwrap_or("This book has no audio tracks.")
                .into());
        };
        let reference = &tracks
            .get(track)
            .ok_or("This book has no audio track there.")?
            .file;
        let file = identifier(reference)?;
        let id = identifier(&item.id)?;
        let path = format!("/api/items/{id}/file/{file}");
        Ok(Playback {
            url: self.api.url(&path),
            headers: vec![self.api.playback_header(&path)?],
        })
    }

    pub(super) fn book_report(
        &self,
        item: &Item,
        _version: &Version,
        progress: Progress,
    ) -> SourceResult<()> {
        let id = identifier(&item.id)?;
        let duration = progress.duration_ms;
        let position = progress.position_ms.min(duration);
        let request = self
            .api
            .patch(&format!("/api/me/progress/{id}"))
            .timeout(Duration::from_secs(4))
            .json(&serde_json::json!({
                "duration": duration as f64 / 1000.0,
                "currentTime": position as f64 / 1000.0,
                "progress": position as f64 / duration.max(1) as f64,
            }));
        crate::http::check_status(self.api.send(request)?.status(), super::SERVER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_fixture::fixture;
    use crate::media::PlaybackState;
    use crate::plugins::{
        OpenOptions,
        audiobookshelf::{PLUGIN, tests::credentials},
    };

    #[test]
    fn track_references_and_progress_stay_on_the_book_timeline() {
        let (origin, received) = fixture(vec![
            ("PATCH /api/me/progress/book", 200, vec![]),
            ("PATCH /api/me/progress/book", 403, vec![]),
        ]);
        let client = (PLUGIN.open)(&credentials(&origin), OpenOptions::default()).unwrap();
        let item = Item {
            id: "book".into(),
            ..Item::default()
        };
        let tracks = serde_json::from_str(r#"[{"ino":"file1","duration":9.125,"contentUrl":"https://untrusted.invalid/path"},{"ino":"file2","duration":2}]"#).unwrap();
        let (version, duration) = version(tracks).unwrap();
        assert_eq!(duration, 11_125);
        let Stream::Audiobook(tracks) = &version.stream else {
            panic!("expected tracks");
        };
        assert_eq!(tracks[1].start_ms, 9125);
        let playback = client.playback(&item, &version, 0).unwrap();
        assert_eq!(
            playback.url.as_str(),
            format!("{origin}/api/items/book/file/file1")
        );
        assert_eq!(
            playback.header_lines(),
            ["authorization: Bearer test-token"]
        );
        let second = client.playback(&item, &version, 1).unwrap();
        assert_eq!(
            second.url.as_str(),
            format!("{origin}/api/items/book/file/file2")
        );
        assert!(client.playback(&item, &version, 2).is_err());
        let progress = || Progress {
            state: PlaybackState::Stopped,
            position_ms: 9500,
            duration_ms: 11125,
        };
        client.report(&item, &version, progress()).unwrap();
        assert!(client.report(&item, &version, progress()).is_err());
        let requests = received.join().unwrap();
        assert_eq!(requests[0].body["currentTime"], 9.5);
        assert_eq!(requests[0].body["duration"], 11.125);
    }

    #[test]
    fn invalid_tracks_never_become_partial_books() {
        for bad in [
            r#"[{"ino":"1","duration":5},{"ino":"../2","duration":10}]"#,
            r#"[{"ino":"1","duration":0}]"#,
        ] {
            assert!(version(serde_json::from_str(bad).unwrap()).is_err());
        }
    }
}
