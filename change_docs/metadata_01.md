# Metadata 01 — album identity, artist entities, and scan priority

Status: Phases 1–3 implemented.

Applies to: `src/library/scan.rs`, `src/library/mod.rs`,
`src/library/db/library.rs`, `src/library/offline/mod.rs`,
`src/screen/albums.rs`, `src/screen/artists.rs`,
`src/screen/album_detail.rs`, `src/screen/artist_detail.rs`,
`src/library/db/global.rs`, and `change_docs/library_database_01.md`.

## 1. Goal

Stop treating one album as several albums just because its tracks have
different track artists. The scanner currently groups albums by the primary
track artist:

```text
(group artist, album title) = (primary_artist(ARTIST), ALBUM)
```

That makes a compilation, soundtrack, duet album, or any album with featured
artists produce one album per artist. It also throws away `ALBUMARTIST`, the
compilation flag, disc numbers, and most MusicBrainz identifiers.

This document defines:

1. the exact order in which metadata is trusted;
2. how a track is assigned to an album;
3. how album display metadata is chosen;
4. which artists become top-level artist entities;
5. the schema changes needed to store the result.

The procedure applies to `ScanMode::Tags`. `ScanMode::Folders` keeps its
explicit `Artist/Album/Track` directory parsing and is not changed by this
plan, except where a folder is used as a fallback signal in tag mode.

## 2. Locked-in decisions

- **Album identity goes from most unique to least unique metadata.**
  MusicBrainz identifiers come first, then album artist, then track artist,
  then folder structure.
- **Top-level Artists entries are album artists only.**
  Track-only and featured artists remain visible on track rows but do not get
  their own artist page yet.
- **When `ALBUMARTIST` is missing and a folder/album contains several distinct
  primary track artists, infer Various Artists.**
- **Folder structure is a fallback only.**
  It is used when MusicBrainz identifiers and album artist are absent.
- **Track artist display preserves the full `ARTIST` credit.**
  `"Alice feat. Bob"` is stored and displayed as `"Alice feat. Bob"`.
  `primary_artist()` is used only for fallback grouping decisions.
- **The compilation artist is a stable internal entity named
  `"Various Artists"`.** It is not localized in this plan.
- **`ALBUMARTIST` is never passed through `primary_artist()`.**
  A collaboration credit such as `"Alice & Bob"` stays `"Alice & Bob"`.

## 3. Raw metadata read per track

The scanner reads this once per audio file:

| Field | Tag key (lofty) | Common native tags | Required |
|---|---|---|---|
| Track title | `ItemKey::TrackTitle`, fallback file stem | `TIT2`, `TITLE`, `©nam` | yes |
| Album title | `ItemKey::AlbumTitle` | `TALB`, `ALBUM`, `©alb` | no |
| Album artist | `ItemKey::AlbumArtist` | `TPE2`, `ALBUMARTIST`, `ALBUM ARTIST`, `aART` | no |
| Album artists (multi) | `ItemKey::AlbumArtists` | `TXXX:ALBUMARTISTS`, `ALBUMARTISTS`, freeform | no, phase 2 |
| Track artist display | `ItemKey::TrackArtist` | `TPE1`, `ARTIST`, `©ART` | no |
| Track artists (multi) | `ItemKey::TrackArtists` | `TXXX:ARTISTS`, `ARTISTS`, freeform | no, future |
| Compilation flag | `ItemKey::FlagCompilation` | `TCMP`, `COMPILATION`, `cpil` | no |
| Track number | `ItemKey::TrackNumber` | `TRCK`, `TRACKNUMBER`, `trkn` | no |
| Disc number | `ItemKey::DiscNumber` | `TPOS`, `DISCNUMBER`, `disk` | no |
| Disc total | `ItemKey::DiscTotal` | `TPOS` total, `DISCTOTAL`, `disk` total | no |
| MusicBrainz release id | `ItemKey::MusicBrainzReleaseId` | `TXXX:MusicBrainz Album Id`, `MUSICBRAINZ_ALBUMID`, freeform | no |
| MusicBrainz release-group id | `ItemKey::MusicBrainzReleaseGroupId` | `TXXX:MusicBrainz Release Group Id`, `MUSICBRAINZ_RELEASEGROUPID`, freeform | no |
| MusicBrainz album artist id | `ItemKey::MusicBrainzReleaseArtistId` | `TXXX:MusicBrainz Album Artist Id`, `MUSICBRAINZ_ALBUMARTISTID`, freeform | no |
| MusicBrainz artist id | `ItemKey::MusicBrainzArtistId` | `TXXX:MusicBrainz Artist Id`, `MUSICBRAINZ_ARTISTID`, freeform | no |
| Album artist sort order | `ItemKey::AlbumArtistSortOrder` | `TSO2`, `ALBUMARTISTSORT`, `soaa` | no |
| Track artist sort order | `ItemKey::TrackArtistSortOrder` | `TSOP`, `ARTISTSORT`, `soar` | no |
| Folder | filesystem path relative to the library root | — | yes |

Lofty already maps the common format aliases to these `ItemKey` values. The
scanner should read the keys rather than format-specific frames.

Missing or empty values become `None`. A tag containing only whitespace is
treated as missing.

### 3.1 Additional fields

- `musicbrainz_albumartistid` is an artist identity field, not part of the
  album identity key. It is used to keep artist entities distinct when names
  collide.
- `musicbrainz_artistid` is stored on the track or used to canonicalize the
  track artist entity. It does not decide album membership.
- `musicbrainz_recordingid`/release-track ids are not needed for album
  grouping. They can be added later for deduplication or scrobbling.
- `ARTISTS`/`ALBUMARTISTS` multi-values are phase 2. Phase 1 uses the literal
  `ARTIST` and `ALBUMARTIST` strings.

## 4. Normalization rules

Normalization is for comparison and key generation only. The original values
are kept for display.

For text values:

1. Trim leading and trailing whitespace.
2. Collapse each run of internal whitespace to one space.
3. Compare case-insensitively using lowercase.
4. Treat an empty result as absent.
5. Do not strip punctuation, accents, or articles. `"The Beatles"` and
   `"Beatles"` are different values.
6. Do not use `primary_artist()` on album artists or album titles.

For MusicBrainz ids:

1. Trim whitespace.
2. Lowercase the UUID.
3. Treat malformed or empty ids as absent.
4. A MusicBrainz id is used only when it is the same across the tracks being
   considered for one album.

For folder paths:

1. Use the path relative to the library root with `/` separators.
2. Do not include the file name.
3. Normalize redundant separators, but keep the path case-sensitive on
   case-sensitive filesystems.
4. A disc subfolder named `CD1`, `Disc 1`, `Disk 1`, or similar may be folded
   into its parent for folder fallback if the parent also carries the album
   title. This avoids splitting multi-disc albums when every other tag is
   missing.

`primary_artist()` remains a fallback helper. The existing `never_split`
configuration still protects names such as `"Simon & Garfunkel"` when that
fallback is used.

## 5. Album identity

### 5.1 Priority order

For tracks being considered for the same release, use the first identity that
is available and consistent. The order is deliberately most-unique first:

| Priority | Identity | Key shape | Notes |
|---|---|---|---|
| 1 | MusicBrainz release id | `mb-release:<uuid>` | Strongest identity. Different releases stay separate. |
| 2 | MusicBrainz release-group id | `mb-release-group:<uuid>` | Used only when the release id is absent. May merge editions/remasters of the same release group. |
| 3 | Album artist + album title | `album-artist:<norm artist>\0album:<norm title>` | Normal case for a consistently tagged local library. |
| 4 | Primary track artist + album title | `artist:<norm primary>\0album:<norm title>` | Only when every track in the candidate has the same non-empty primary track artist. |
| 5 | Folder + album title | `folder:<relative folder>\0album:<norm title>` | Offline fallback when tags are missing. |
| 6 | Folder alone | `folder:<relative folder>` | Last resort when even the album title is missing. |
| 7 | Various Artists / Unknown | `various-artists\0album:<norm title>` or `unknown-artist\0album:<norm title>` | Used when artists are absent but the album can still be identified. |

The priority is applied to the whole release candidate, not independently to
each track. A track without a MusicBrainz id must not split away from the rest
of its album just because another track in the same folder has an id.

### 5.2 Initial candidate set

The scanner first forms a candidate set of tracks that could belong to one
album. This is necessary because a track artist fallback or a Various Artists
inference needs to look at the other tracks in the release.

1. **Strong-id candidates.** Tracks with the same non-empty MusicBrainz
   release id form one candidate across the whole library. This allows an
   album spread across `CD1`/`CD2` folders to stay together.
2. **Release-group candidates.** Remaining tracks with the same non-empty
   release-group id form one candidate.
3. **Album-artist candidates.** Remaining tracks with the same normalized
   album artist + normalized album title form one candidate across the
   library.
4. **Folder/artist candidates.** Remaining tracks are considered together when
   they share:
   - the same relative folder, and
   - the same normalized album title, or the folder has no album title.
5. **Unmatched tracks.** A track that has no album, no artist, and no usable
   folder goes into an `Unknown Album` group.

A track that shares a strong id with an existing candidate joins that
candidate. A track without a strong id may still join a strong-id candidate
when it shares the same folder and normalized album title; otherwise it stays
in its fallback candidate. This mixed-tag case is an explicit edge case in
§9.

### 5.3 Resolving a candidate

Once a candidate set exists, resolve its identity in this order:

1. If all tracks that have a MusicBrainz release id agree on one release id,
   use `mb-release:<uuid>`.
2. Otherwise, if all tracks that have a MusicBrainz release-group id agree on
   one release-group id, use `mb-release-group:<uuid>`.
3. Otherwise, if the candidate has a non-empty, consistent album artist, use
   normalized album artist + normalized album title.
4. Otherwise, compute the primary track artist for every track:
   - If every track has the same non-empty primary artist, use that artist +
     album title.
   - If the candidate has no album artist, has more than one distinct primary
     artist, or `COMPILATION=1`, use `various-artists` + album title and mark
     the album as a compilation.
5. Otherwise, use the relative folder + album title.
6. Otherwise, use the relative folder alone.
7. Otherwise, use `Unknown Artist` + `Unknown Album`.

The album title used in the key is the literal `ALBUM` tag after normalization.
If `ALBUM` is missing, use the last non-empty folder component as the display
title. If there is no usable folder, use `"Unknown Album"`.

`COMPILATION=1` is a hint, not an override. If `ALBUMARTIST` is explicitly
present, it wins over the compilation flag. The flag only selects Various
Artists when no album artist exists and the candidate otherwise looks like a
multi-artist release.

Accepted true values for the compilation flag are `1`, `true`, `yes`, and
`on`, case-insensitive. `0`, `false`, `no`, `off`, and empty mean false.

### 5.4 Album display metadata

Identity and display are related but separate. The display values are chosen
as follows:

| Display field | Priority |
|---|---|
| Album title | `ALBUM` tag → folder name → `"Unknown Album"` |
| Album artist | `ALBUMARTIST` as written → joined `ALBUMARTISTS` entries → consistent primary track artist → `"Various Artists"` → `"Unknown Artist"` |
| Compilation | true when the identity resolved to Various Artists, or when `COMPILATION` is true and no explicit album artist overrides it |
| Disc count | maximum `DISCNUMBER`/`DISCTOTAL` seen in the album; at least 1 |
| Album sort name | `ALBUMARTISTSORT`/album sort tag when present → display artist |

The album artist display string is kept whole. If `ALBUMARTIST` is
`"Alice & Bob"`, the display string is `"Alice & Bob"`. It is not rewritten to
`"Alice"`. If `ALBUMARTISTS` exists, the individual names may be stored as
separate album-artist entities, while the display string remains the literal
`ALBUMARTIST` credit.

### 5.5 Track metadata

A track keeps its own metadata independently of album identity:

| Display field | Priority |
|---|---|
| Track title | `TITLE` tag → file stem |
| Track artist | full `ARTIST` value → album artist → `"Unknown Artist"` |
| Track number | `TRACKNUMBER`/`TRACK` tag |
| Disc number | `DISCNUMBER`/`DISK` tag, default 1 |
| Album id | resolved album identity from §5.3 |
| Album title | resolved album title from §5.4 |
| Cover | unchanged by this plan |

Track artist display is never collapsed to the primary artist. For example,
`"Kendrick Lamar feat. GLC"` remains that full string. `primary_artist()` is
only used to decide whether the tracks in an untagged candidate all share one
performer for album grouping.

Album tracks are ordered by:

1. disc number (missing → 1),
2. track number (missing → after numbered tracks),
3. title,
4. track id.

## 6. Artist entities

### 6.1 What creates a top-level artist

The Artists screen is driven by **album artists only**. An artist gets a
top-level entry when it is credited as an album artist on at least one album.

The following create album-artist membership:

1. `ALBUMARTISTS` multi-values, when present. Each value becomes an album
   artist.
2. Otherwise, the literal `ALBUMARTIST` value. It is stored whole and is not
   split on `&`, `,`, or `feat.`.
3. Otherwise, the consistent primary track artist used by the album fallback.
4. `"Various Artists"` when the album resolved to the compilation identity.
5. `"Unknown Artist"` when no artist could be determined at all.

Track-only artists do not get a top-level Artists entry in this plan. They may
still exist in the `artists` table so that `tracks.artist_id` remains valid,
but the Artists query filters to artists that participate in
`album_artists`.

### 6.2 Artist identity

Each artist row has:

- a stable id;
- a display name;
- an optional sort name;
- an optional MusicBrainz artist id;
- an album count.

Identity priority for an artist entity:

1. MusicBrainz artist id / album-artist id when present.
2. Normalized display name otherwise.

The id format should make the source explicit, for example:

```text
mb:<uuid>
name:<normalized-name>
```

The manager still prefixes ids with the library identifier on the way out.
Display names are preserved exactly as tagged; only the id uses normalization.

Two artists with the same display name but different MusicBrainz ids remain
separate. Two spellings of the same artist without ids merge only when their
normalized names match exactly.

### 6.3 Album artist association

An album can have more than one album artist. Store the association in a join
table rather than packing it into `albums.artist_name`:

```text
album_artists
  album_id  TEXT NOT NULL REFERENCES albums(id) ON DELETE CASCADE
  artist_id TEXT NOT NULL REFERENCES artists(id) ON DELETE CASCADE
  position  INTEGER NOT NULL
  PRIMARY KEY (album_id, artist_id)
```

`albums.artist_name` remains the display credit and `albums.artist_id` remains
the primary display artist for existing UI code. The join table is the source
of truth for:

- the Artists screen;
- `Artist.album_count`;
- `ArtistDetail.albums`;
- “albums by this artist” queries.

The `artist_id` in `album_artists` for a Various Artists album is the stable
id `various-artists`, whose display name is `"Various Artists"`.

## 7. Database changes

Bump `SCHEMA_VERSION` and `SCANNER_VERSION`. The schema change is destructive
in the existing design: the library database is derived data and will be
recreated, then rescanned. Playlists and history live in the global database
and are unaffected; any dangling track snapshots are already handled by the
playlist UI rules.

### 7.1 `artists`

| Column | Change | Purpose |
|---|---|---|
| `id` | existing, new id scheme | Stable entity id (`mb:` or `name:`). |
| `name` | existing | Display name. |
| `sort_name` | existing | From sort tag when present, else name. |
| `mb_artist_id` | new, nullable | MusicBrainz artist id. |
| `album_count` | existing | Count of rows in `album_artists`. |
| `cover_id` | existing | No change. |

Add an index on `mb_artist_id`.

### 7.2 `albums`

| Column | Change | Purpose |
|---|---|---|
| `id` | existing, generated from §5.3 | Album identity key. |
| `name` | existing | Display title. |
| `artist_id` | existing | Primary display artist. |
| `artist_name` | existing | Display credit, may contain `&`, `feat.`, etc. |
| `song_count` | existing | Track count. |
| `duration_ms` | existing | Total duration. |
| `cover_id` | existing | No change. |
| `compilation` | new `INTEGER NOT NULL DEFAULT 0` | Whether this is a Various Artists album. |
| `mb_release_id` | new, nullable | MusicBrainz release id. |
| `mb_release_group_id` | new, nullable | MusicBrainz release-group id. |
| `disc_count` | new, nullable | Maximum disc number, at least 1 when known. |
| `album_sort_name` | new, nullable | Album artist sort / album sort name. |

Add indexes on `mb_release_id`, `mb_release_group_id`, and `artist_id`.

### 7.3 `tracks`

| Column | Change | Purpose |
|---|---|---|
| `id` | existing | Offline: relative path. |
| `title` | existing | Track title. |
| `artist_id` | existing | Track artist entity; may be track-only. |
| `artist_name` | existing | Full `ARTIST` display credit. |
| `album_id` | existing | Resolved album identity. |
| `album_name` | existing | Resolved album title. |
| `track_no` | existing | Track number. |
| `disc_no` | new, nullable | Disc number for ordering and display. |
| `duration_ms` | existing | No change. |
| `cover_id` | existing | No change. |

Add an index on `(album_id, disc_no, track_no)` or keep the existing
`tracks_album_idx` plus a composite index if query plans require it.

Do **not** add a `compilation` column to `tracks`; compilation is an album
property.

### 7.4 `album_artists`

New table as shown in §6.3. Add an index on `artist_id`. The
`PRIMARY KEY (album_id, artist_id)` gives the reverse lookup from album to
artists.

### 7.5 Query changes

- `LibraryDb::artists()` selects only artists that have at least one
  `album_artists` row.
- `LibraryDb::artist()` selects albums through `album_artists`, not through
  `albums.artist_id`, so collaboration albums appear under every album artist.
- `LibraryDb::albums()` and `LibraryDb::album()` read `artist_name` as the
  display credit.
- `LibraryDb::album()` orders tracks by `disc_no`, `track_no`, then title/id.
- `album_count` is recomputed from `album_artists` during the scan write.

## 8. Procedure summary

The whole scan procedure in order:

1. Walk the library and read each file's tags once.
2. For every track, capture title, album title, album artist, album artists,
   full track artist, compilation flag, disc/track numbers, MusicBrainz ids,
   and relative folder.
3. Normalize only for comparison and ids; keep display strings untouched.
4. Form release candidates:
   - MusicBrainz release id first;
   - then release-group id;
   - then album artist + album title;
   - then folder + album title for the remaining ambiguous tracks.
5. Resolve each candidate to one album identity using the priority in §5.3.
6. Choose the album display artist:
   - explicit album artist;
   - else consistent primary track artist;
   - else Various Artists;
   - else Unknown Artist.
7. Choose album title and disc count.
8. Create an album row and one `album_artists` row per album artist.
9. Assign every track to its album, preserving the full track artist credit
   and storing its disc/track number.
10. Create top-level artist entities only for album artists. Track-only
    artists may exist in `artists` for foreign keys but are excluded from the
    Artists screen.
11. Write everything in the existing short transaction, then bump the scan
    revision.

## 9. Examples

### Example A — soundtrack with a proper album artist

```text
Track 1: ARTIST=Alice,  ALBUMARTIST=Various Artists, ALBUM=Soundtrack, COMPILATION=1
Track 2: ARTIST=Bob,    ALBUMARTIST=Various Artists, ALBUM=Soundtrack, COMPILATION=1
```

Result: one album, display artist `"Various Artists"`, two track rows with
different track artists.

### Example B — compilation with no album artist

```text
Track 1: ARTIST=Alice, ALBUM=Hits, COMPILATION=1, folder=Hits
Track 2: ARTIST=Bob,   ALBUM=Hits, COMPILATION=1, folder=Hits
```

Result: one Various Artists album keyed by `various-artists + Hits`.

### Example C — solo album with featured artists and no album artist

```text
Track 1: ARTIST=Alice feat. Bob,   ALBUM=Show, folder=Show
Track 2: ARTIST=Alice feat. Carol, ALBUM=Show, folder=Show
```

`primary_artist()` gives `"Alice"` for every track. Result: one album by
`"Alice"`. Track rows still show the full featured credits.

### Example D — untagged compilation folder

```text
Track 1: ARTIST=Alice, ALBUM= (missing), folder=Hits
Track 2: ARTIST=Bob,   ALBUM= (missing), folder=Hits
```

Result: one album with display title `"Hits"`, album artist
`"Various Artists"`, keyed by folder + album title.

### Example E — same album title by different artists

```text
Folder A: ARTIST=Alice, ALBUM=Greatest Hits
Folder B: ARTIST=Bob,   ALBUM=Greatest Hits
```

Result: two albums, because the album-artist/artist keys differ and the folders
differ.

### Example F — collaboration album with explicit multi-values

```text
ALBUMARTIST=Alice & Bob
ALBUMARTISTS=Alice
ALBUMARTISTS=Bob
ALBUM=Together
```

Result: one album with display artist `"Alice & Bob"`, and both `"Alice"` and
`"Bob"` appear on the Artists screen through `album_artists`.

### Example G — MusicBrainz-tagged multi-disc release

```text
All tracks: MUSICBRAINZ_ALBUMID=<same uuid>
Disc 1 and Disc 2 live in separate folders
```

Result: one album keyed by `mb-release:<uuid>`, with `disc_count = 2` and
tracks ordered by disc then track number.

### Example H — mixed tagged/untagged siblings

```text
Track 1: MUSICBRAINZ_ALBUMID=<uuid>, folder=Album
Track 2: no MusicBrainz id,         folder=Album, ALBUMARTIST=Alice, ALBUM=Album
```

Track 2 joins the release if it shares the same folder and normalized album
title. If its album artist conflicts with the release, it stays in its own
fallback group and is reported as an edge case in scan logs. The scanner must
not silently merge an explicit conflicting album artist into a different
artist's release.

## 10. Tests

The identity logic should be a pure function over `RawTrack` values so it can
be tested without SQLite, lofty, or the filesystem.

Required cases:

1. Different track artists + same album artist → one album.
2. Collaboration album with two album artists → one album, two artist links.
3. Compilation flag + no album artist → one Various Artists album.
4. No album artist + differing primary artists in one folder → Various
   Artists.
5. No album artist + same primary artist with different featured credits →
   one album by the primary artist.
6. Same album title by different artists in different folders → separate
   albums.
7. Same MusicBrainz release id across two folders → one album.
8. Same release-group id but different release ids → one album by
   release-group priority.
9. Multi-disc album → one album, tracks ordered disc then track.
10. Missing tags everywhere → folder fallback, then `Unknown Album`.
11. `ALBUMARTIST=Simon & Garfunkel` and `never_split` → not split.
12. `ALBUMARTIST=Alice & Bob` → display credit preserved, not collapsed.
13. Mixed tagged/untagged siblings → compatible track joins the release;
    conflicting album artist does not.

Add at least one fixture-based scan test that writes real tags with lofty and
verifies the resulting `ScanData` rows.

## 11. Implementation phases

### Phase 1 — identity and display artist

- Read album artist, compilation flag, MusicBrainz release/release-group ids.
- Implement candidate building and the priority resolver.
- Preserve full track artist display.
- Store `compilation`, `mb_release_id`, `mb_release_group_id`, and
  `disc_count` on albums.
- Infer Various Artists when required.
- Keep one album-artist association per album for now.
- Bump schema/scanner versions and rescan.

Phase 1 implementation notes:

- The pure resolver lives in `src/library/identity.rs`.
- Until `album_artists` exists, `albums.artist_id` is the single
  album-artist association. The Artists screen filters on
  `artists.album_count > 0`, so track-only artists stay in the table for
  foreign keys but do not appear as top-level entries.
- `tracks.disc_no` and disc-aware ordering are deferred to Phase 2; Phase 1
  still reads disc tags to compute `albums.disc_count`.

### Phase 2 — multiple album artists and discs

- Read `ALBUMARTISTS` multi-values.
- Add `album_artists`.
- Add `tracks.disc_no`.
- Order album tracks by disc then track.
- Expose multiple album artists through `ArtistDetail`.

Phase 2 implementation notes:

- `MUSICBRAINZ_ALBUMARTISTID` is read as a multi-value list and matched to
  `ALBUMARTISTS` by position. Missing ids fall back to `name:<normalized>`.
- Identity key for multi-value credits sorts the normalized names and joins
  them with `\u{1f}` so track order does not split an album; the display
  credit joins the literal spellings with `" & "`.
- `album_artists` is the source of truth for Artists-screen membership,
  `album_count`, and artist-detail albums. `albums.artist_id` is retained as
  the primary artist for compatibility.
- `tracks.disc_no` is stored and album detail orders by
  `disc_no`, then `track_no`.
- Schema/scanner versions are `3`.

### Phase 3 — UI and polish

- Show the compilation badge / Various Artists collection.
- Show disc headers when `disc_count > 1`.
- Show full track artist credits consistently.
- Optionally add structured `ARTISTS` support for future featured-artist
  pages and search.

Phase 3 implementation notes:

- Album detail adds `"Compilation"` and the disc count to the header metadata.
- Multi-disc albums interleave fixed-height `Disc N` header rows in the same
  lazy list as the tracks, so virtualization keeps working.
- Track rows keep displaying the full `ARTIST` credit stored on the track.

## 12. Out of scope

- Writing tags back to files.
- Automatically looking up missing MusicBrainz data online.
- AcoustID fingerprint matching.
- Merging deluxe/remaster editions beyond the release-group rule.
- Featured-artist pages.
- Localized `"Various Artists"` display names.
- Changing `ScanMode::Folders` behavior.
- Changing cover-art selection.
