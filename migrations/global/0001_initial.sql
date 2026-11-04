CREATE TABLE playlists (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_id    TEXT NOT NULL,
    library_id  TEXT NOT NULL,
    title       TEXT,
    artist      TEXT,
    album       TEXT,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);

CREATE TABLE play_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    TEXT NOT NULL,
    library_id  TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    played_ms   INTEGER,
    completed   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX play_history_started_idx ON play_history(started_at);
