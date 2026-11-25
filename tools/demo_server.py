"""Standard-library-only Plex fixture. Synthetic media, no real credentials."""

import io
import json
import math
import struct
import threading
import wave
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

PROGRESS = {}
LOCK = threading.Lock()
NAMES = [
    "The Quiet Orbit", "Tidal Light", "After the Rain", "A Place Between",
    "Last Light", "Northbound", "Slow Motion", "The Long Way Home",
    "Still Water", "A Small Discovery", "Parallel Lines", "Night Windows",
]


def poster(index):
    width, height = 240, 360
    colors = [(33, 96, 103), (135, 74, 58), (55, 76, 121), (89, 70, 114)]
    red, green, blue = colors[index % len(colors)]
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            glow = max(0, 1 - math.hypot(x - 120, y - 130) / 190)
            stripe = 28 if abs((x + index * 20) % 90 - y // 3) < 6 else 0
            rows.extend(min(255, int(c * (0.4 + glow * 0.8)) + stripe) for c in (red, green, blue))

    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">2I5B", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


POSTERS = [poster(i) for i in range(4)]
wav_buffer = io.BytesIO()
with wave.open(wav_buffer, "wb") as audio:
    audio.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
    audio.writeframes(b"".join(struct.pack("<h", int(900 * math.sin(2 * math.pi * 220 * n / 16000))) for n in range(16_000 * 20)))
AUDIO = wav_buffer.getvalue()
long_buffer = io.BytesIO()
with wave.open(io.BytesIO(AUDIO), "rb") as source, wave.open(long_buffer, "wb") as audio:
    audio.setparams(source.getparams())
    audio.writeframes(source.readframes(source.getnframes()) * 24)
LONG_AUDIO = long_buffer.getvalue()


def item(index, kind="movie"):
    rating_key = str(index)
    with LOCK:
        offset = PROGRESS.get(rating_key, 0)
    return {
        "ratingKey": rating_key,
        "title": NAMES[(index - 1) % len(NAMES)] + (f" · {index}" if index > 12 else ""),
        "type": kind,
        "year": 2020 + index % 6,
        "originallyAvailableAt": f"{2020 + index % 6}-{index % 12 + 1:02d}-{index % 28 + 1:02d}",
        "summary": "A synthetic library item for trying Perfuse. This fixture exercises browsing, artwork, media selection, playback, and saved progress without connecting to a real Plex server.",
        "thumb": f"/library/metadata/{index}/thumb",
        "art": f"/library/metadata/{index}/art",
        "duration": 480000 if index == 200 else 20000,
        "viewOffset": offset,
        "viewCount": int(offset >= 19000),
        "Media": [{"audioCodec": "pcm_s16le", "container": "wav", "Part": [{"key": f"/library/parts/{index}/file.wav"}]}] if kind in ("movie", "episode") else [],
    }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass  # Never log headers or request URLs containing potential credentials.

    def respond(self, body, content_type="application/json", status=200):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if self.headers.get("X-Plex-Token") != "demo":
            self.respond(b"{}", status=401)
            return
        parsed = urlsplit(self.path)
        path, query = parsed.path, parse_qs(parsed.query)
        if path == "/library/sections":
            container = {"Directory": [
                {"key": "1", "title": "Movies", "type": "movie"},
                {"key": "2", "title": "TV Shows", "type": "show"},
            ]}
        elif path == "/photo/:/transcode":
            source = query.get("url", [""])[0].split("/")
            index = int(source[3]) if len(source) > 3 and source[3].isdigit() else 0
            self.respond(POSTERS[index % 4], "image/png")
            return
        elif path.startswith("/library/parts/"):
            self.respond(LONG_AUDIO if path == "/library/parts/200/file.wav" else AUDIO, "audio/wav")
            return
        elif path == "/:/timeline":
            with LOCK:
                PROGRESS[query["ratingKey"][0]] = int(query["time"][0])
            self.respond(b"{}")
            return
        else:
            if path == "/library/recentlyAdded":
                items = [item(index) for index in range(42, 0, -1)]
            elif path == "/library/onDeck":
                items = [item(index) for index in range(1, 43)]
                items = [entry for entry in items if 0 < entry["viewOffset"] < entry["duration"]]
            elif path == "/library/sections/1/all":
                items = [item(index) for index in range(1, 43)]
            elif path == "/library/sections/2/all" and query.get("type") == ["4"]:
                episode = item(102, "episode")
                episode.update(title="First Contact", parentIndex=1, index=1, grandparentTitle="Signals")
                items = [episode]
            elif path == "/library/sections/2/all":
                show = item(100, "show")
                show["title"] = "Signals"
                items = [show]
            elif path == "/library/metadata/100/children":
                season = item(101, "season")
                season["title"] = "Season 1"
                items = [season]
            elif path == "/library/metadata/101/children":
                episode = item(102, "episode")
                episode.update(title="First Contact", parentIndex=1, index=1, grandparentTitle="Signals")
                items = [episode]
            elif path.startswith("/library/metadata/") and path.rsplit("/", 1)[1].isdigit():
                index = int(path.rsplit("/", 1)[1])
                entry = item(index, "episode" if index == 102 else "movie")
                if index == 102:
                    entry.update(title="First Contact", parentIndex=1, index=1, grandparentTitle="Signals")
                items = [entry]
            else:
                self.respond(b"{}", status=404)
                return
            search = query.get("title", [""])[0].casefold()
            items = [entry for entry in items if search in entry["title"].casefold()]
            if query.get("sort") == ["originallyAvailableAt:desc"]:
                items.sort(key=lambda entry: entry["originallyAvailableAt"], reverse=True)
            offset = int(self.headers.get("X-Plex-Container-Start", 0))
            size = int(self.headers.get("X-Plex-Container-Size", 36))
            container = {"Metadata": items[offset:offset + size], "totalSize": len(items), "offset": offset}
        self.respond(json.dumps({"MediaContainer": container}).encode())


if __name__ == "__main__":
    print("Perfuse fixture: http://127.0.0.1:32499 · token: demo", flush=True)
    ThreadingHTTPServer(("127.0.0.1", 32499), Handler).serve_forever()
