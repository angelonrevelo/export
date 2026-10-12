/// A parsed HLS media playlist.
///
/// Segment boundaries are preserved. [`spans`](MediaPlaylist::spans) collapses
/// adjacent byte ranges of the same URI when the hole between them is at most
/// [`COALESCE_GAP_BYTES`](crate::COALESCE_GAP_BYTES).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    pub version: Option<u32>,
    pub target_duration: Option<u64>,
    pub media_sequence: u64,
    pub discontinuity_sequence: u64,
    pub playlist_type: Option<PlaylistType>,
    pub independent_segments: bool,
    pub i_frames_only: bool,
    pub end_list: bool,
    /// First `#EXT-X-MAP`, when the playlist has an initialization section.
    pub init: Option<Map>,
    pub segments: Vec<Segment>,
    pub spans: Vec<IndexedSpan>,
}

/// `EVENT` or `VOD` from `#EXT-X-PLAYLIST-TYPE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistType {
    Event,
    Vod,
}

/// One `#EXTINF` media segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub uri: String,
    pub duration: f64,
    pub title: String,
    pub byte_range: Option<ByteRange>,
    pub discontinuity: bool,
    pub gap: bool,
    pub sequence: u64,
    pub program_date_time: Option<String>,
    /// Initialization map in force for this segment.
    pub map: Option<Map>,
}

/// Inclusive-start byte range of `length` bytes at `offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub length: u64,
    pub offset: u64,
}

impl ByteRange {
    /// First byte past this range.
    pub fn end(self) -> Option<u64> {
        self.offset.checked_add(self.length)
    }
}

/// `#EXT-X-MAP` initialization section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    pub uri: String,
    pub byte_range: Option<ByteRange>,
}

/// Contiguous byte range of one URI after coalescing.
///
/// `byte_range == None` means the whole resource (the segment had no
/// `#EXT-X-BYTERANGE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedSpan {
    pub uri: String,
    pub byte_range: Option<ByteRange>,
    /// Playlist segment indexes covered by this span, in ascending order.
    pub segments: Vec<usize>,
}
