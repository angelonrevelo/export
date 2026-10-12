use crate::model::{ByteRange, IndexedSpan, Segment};

/// Holes of this many bytes or fewer are absorbed into one byte range.
pub const COALESCE_GAP_BYTES: u64 = 4096;

/// Merge byte ranges whose gap is at most [`COALESCE_GAP_BYTES`].
///
/// Ranges are sorted by offset. Overlaps merge. The merged range covers the
/// hole, so a later fetch can issue one contiguous read.
pub fn coalesce_byte_ranges(ranges: &[ByteRange]) -> Vec<ByteRange> {
    let mut ordered = ranges.to_vec();
    ordered.sort_by(|a, b| a.offset.cmp(&b.offset).then(a.length.cmp(&b.length)));
    let mut out: Vec<ByteRange> = Vec::new();
    for range in ordered {
        if let Some(cur) = out.last_mut() {
            if let Some(merged) = merge_if_close(*cur, range) {
                *cur = merged;
                continue;
            }
        }
        out.push(range);
    }
    out
}

/// Group segment byte ranges by URI and coalesce each group.
///
/// Segments with no byte range share one whole-resource span per URI.
pub fn index_segments(segments: &[Segment]) -> Vec<IndexedSpan> {
    let mut groups: Vec<Group> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let position = groups.iter().position(|group| group.uri == segment.uri);
        let group = if let Some(position) = position {
            &mut groups[position]
        } else {
            groups.push(Group {
                uri: segment.uri.clone(),
                ranges: Vec::new(),
                wholes: Vec::new(),
            });
            groups.last_mut().expect("group was just pushed")
        };
        match segment.byte_range {
            Some(range) => group.ranges.push((range, index)),
            None => group.wholes.push(index),
        }
    }

    let mut spans = Vec::new();
    for group in groups {
        if !group.wholes.is_empty() {
            spans.push(IndexedSpan {
                uri: group.uri.clone(),
                byte_range: None,
                segments: group.wholes,
            });
        }
        for (range, mut indexes) in coalesce_indexed(group.ranges) {
            indexes.sort_unstable();
            spans.push(IndexedSpan {
                uri: group.uri.clone(),
                byte_range: Some(range),
                segments: indexes,
            });
        }
    }
    spans.sort_by_key(|span| span.segments.iter().copied().min().unwrap_or(usize::MAX));
    spans
}

struct Group {
    uri: String,
    ranges: Vec<(ByteRange, usize)>,
    wholes: Vec<usize>,
}

fn coalesce_indexed(mut items: Vec<(ByteRange, usize)>) -> Vec<(ByteRange, Vec<usize>)> {
    items.sort_by(|a, b| a.0.offset.cmp(&b.0.offset).then(a.1.cmp(&b.1)));
    let mut out: Vec<(ByteRange, Vec<usize>)> = Vec::new();
    for (range, index) in items {
        if let Some((cur, indexes)) = out.last_mut() {
            if let Some(merged) = merge_if_close(*cur, range) {
                *cur = merged;
                indexes.push(index);
                continue;
            }
        }
        out.push((range, vec![index]));
    }
    out
}

fn merge_if_close(left: ByteRange, right: ByteRange) -> Option<ByteRange> {
    let left_end = left.end()?;
    let right_end = right.end()?;
    let start = left.offset.min(right.offset);
    let end = left_end.max(right_end);
    let gap = if right.offset > left_end {
        right.offset - left_end
    } else {
        left.offset.saturating_sub(right_end)
    };
    if gap > COALESCE_GAP_BYTES {
        return None;
    }
    Some(ByteRange {
        offset: start,
        length: end - start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_of_4096_merges_and_covers_the_hole() {
        let merged = coalesce_byte_ranges(&[
            ByteRange {
                offset: 0,
                length: 100,
            },
            ByteRange {
                offset: 100 + 4096,
                length: 50,
            },
        ]);
        assert_eq!(
            merged,
            vec![ByteRange {
                offset: 0,
                length: 100 + 4096 + 50,
            }]
        );
    }

    #[test]
    fn gap_of_4097_stays_split() {
        let merged = coalesce_byte_ranges(&[
            ByteRange {
                offset: 0,
                length: 100,
            },
            ByteRange {
                offset: 100 + 4097,
                length: 50,
            },
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].length, 100);
        assert_eq!(merged[1].offset, 4197);
    }

    #[test]
    fn overlap_merges_without_growing_past_the_union() {
        let merged = coalesce_byte_ranges(&[
            ByteRange {
                offset: 0,
                length: 2000,
            },
            ByteRange {
                offset: 500,
                length: 100,
            },
        ]);
        assert_eq!(
            merged,
            vec![ByteRange {
                offset: 0,
                length: 2000,
            }]
        );
    }
}
