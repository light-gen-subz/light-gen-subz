use std::ops::Range;

use crate::pipeline::stt::Segment;

const MAX_DURATION_SECS: f64 = 7.0;
const MAX_CHARS: usize = 84;
/// A cue shorter than this is unreadable; it is stretched when the next cue allows it.
const MIN_DURATION_SECS: f64 = 0.6;
/// Comfortable reading speed, in display columns per second.
const MAX_READING_SPEED: f64 = 17.0;
/// Room left between two cues when one is stretched.
const MIN_GAP_SECS: f64 = 0.04;
/// A silence this long between two words always ends the cue: it would stay on screen
/// while nobody speaks.
const LONG_PAUSE_SECS: f64 = 1.2;
/// Longest displayed line, in columns (a full-width character takes two).
pub const LATIN_LINE_COLUMNS: usize = 42;
/// Same for text with full-width characters, which are read more slowly.
pub const WIDE_LINE_COLUMNS: usize = 32;

/// Characters that take two columns: CJK ideographs, kana, Hangul and full-width forms.
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// Scripts written without spaces between words: a line may break after any character.
pub fn is_unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F
        | 0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF)
}

/// Columns a text takes on screen.
pub fn display_width(text: &str) -> usize {
    text.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

/// Longest line for `text`: shorter for full-width scripts.
pub fn line_limit(text: &str) -> usize {
    if text.chars().any(is_wide) {
        WIDE_LINE_COLUMNS
    } else {
        LATIN_LINE_COLUMNS
    }
}

/// `max_chars` (stated for Latin text) scaled down when `text` has full-width characters.
fn column_limit(text: &str, max_chars: usize) -> usize {
    if text.chars().any(is_wide) {
        max_chars * WIDE_LINE_COLUMNS / LATIN_LINE_COLUMNS
    } else {
        max_chars
    }
}

/// Joins words with a space, except where a script is written without them.
pub fn join_words<'a>(words: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for word in words {
        let glued = match (out.chars().last(), word.chars().next()) {
            (Some(a), Some(b)) => is_unspaced(a) || is_unspaced(b),
            _ => true,
        };
        if !glued {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// A cue's lines as a single line of text.
pub fn flatten_lines(text: &str) -> String {
    join_words(text.lines().map(str::trim).filter(|l| !l.is_empty()))
}

/// A piece of text a line may break before: a word, or one character of an unspaced script.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub text: String,
    pub space_before: bool,
}

/// Splits `text` into [`Unit`]s, remembering where the original had spaces.
pub fn units_of(text: &str) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    for (i, word) in text.split_whitespace().enumerate() {
        let mut space = i > 0;
        let mut run = String::new();
        let mut run_space = space;
        for c in word.chars() {
            if is_unspaced(c) {
                if !run.is_empty() {
                    units.push(Unit {
                        text: std::mem::take(&mut run),
                        space_before: run_space,
                    });
                    space = false;
                }
                units.push(Unit {
                    text: c.to_string(),
                    space_before: space,
                });
                space = false;
                run_space = false;
            } else {
                if run.is_empty() {
                    run_space = space;
                }
                run.push(c);
            }
        }
        if !run.is_empty() {
            units.push(Unit {
                text: run,
                space_before: run_space,
            });
        }
    }
    units
}

/// The text of a run of units, with the spaces the original had.
pub fn render_units(units: &[Unit]) -> String {
    let mut out = String::new();
    for (i, unit) in units.iter().enumerate() {
        if i > 0 && unit.space_before {
            out.push(' ');
        }
        out.push_str(&unit.text);
    }
    out
}

/// Turns the raw segments of the STT engine into readable subtitle cues: drops empty
/// segments, splits those that are too long (in duration or width) spreading the time
/// in proportion to the characters, then puts the timings back in order.
pub fn build_cues(segments: &[Segment]) -> Vec<Segment> {
    let cleaned: Vec<Segment> = segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| Segment {
            start: s.start,
            end: s.end.max(s.start),
            text: s.text.trim().to_string(),
        })
        .collect();

    let mut cues: Vec<Segment> = cleaned.iter().flat_map(split_if_too_long).collect();
    tidy_timings(&mut cues);
    cues
}

/// Puts cues in order, removes overlaps, and gives each one time to be read: at least
/// [`MIN_DURATION_SECS`], and enough for [`MAX_READING_SPEED`] when the next cue allows it.
fn tidy_timings(cues: &mut [Segment]) {
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    for i in 1..cues.len() {
        if cues[i].start < cues[i - 1].end {
            cues[i - 1].end = cues[i].start.max(cues[i - 1].start);
        }
    }
    for i in 0..cues.len() {
        let limit = cues
            .get(i + 1)
            .map_or(f64::INFINITY, |next| next.start - MIN_GAP_SECS);
        let reading =
            (display_width(&cues[i].text) as f64 / MAX_READING_SPEED).min(MAX_DURATION_SECS);
        let wanted = cues[i].start + reading.max(MIN_DURATION_SECS);
        if cues[i].end < wanted {
            cues[i].end = wanted.min(limit).max(cues[i].end);
        }
    }
}

/// A place where a line may be cut, after one unit.
#[derive(Debug, Clone, Copy)]
struct Item {
    /// 2 after a sentence end, 1 after a clause break, 0 otherwise.
    rank: u8,
    /// Silence between this unit and the next, in seconds.
    pause: f64,
    /// The next unit is closing punctuation, which must not start a line.
    next_closes: bool,
}

fn break_rank(unit: &str) -> u8 {
    if unit.ends_with(['.', '?', '!', '…', '。', '？', '！', '．']) {
        2
    } else if unit.ends_with([',', ';', ':', '—', '，', '；', '：', '、']) {
        1
    } else {
        0
    }
}

fn closes_a_phrase(unit: &str) -> bool {
    unit.chars()
        .next()
        .is_some_and(|c| "。、，．？！；：）」』】〉》”’,.;:?!)]}…".contains(c))
}

fn items_for(texts: &[&str], pauses: impl Fn(usize) -> f64) -> Vec<Item> {
    (0..texts.len())
        .map(|i| Item {
            rank: break_rank(texts[i]),
            pause: pauses(i),
            next_closes: texts.get(i + 1).is_some_and(|t| closes_a_phrase(t)),
        })
        .collect()
}

/// Positions (indices of the first unit of each following piece) that split `items` into
/// `n` pieces of about equal size. Each cut aims at the ideal position but slides up to a
/// third of a piece toward a better place — after a sentence end, a clause break or a real
/// pause in the speech — before or after the target.
fn chunk_cuts(items: &[Item], n: usize) -> Vec<usize> {
    let len = items.len();
    let n = n.clamp(1, len.max(1));
    let target = len as f64 / n as f64;
    let window = ((target / 3.0).floor() as usize).max(1);

    let score = |c: usize, ideal: usize| -> Option<f64> {
        let item = items[c - 1];
        if item.next_closes {
            return None;
        }
        let bonus = f64::from(item.rank) * 0.3 + (item.pause / 0.3).min(4.0) * 0.5;
        (bonus > 0.0).then(|| c.abs_diff(ideal) as f64 / window as f64 - bonus)
    };

    let mut cuts = Vec::with_capacity(n.saturating_sub(1));
    let mut previous = 0usize;
    for k in 1..n {
        let ideal = (k as f64 * target).round() as usize;
        // Leave at least one unit to every piece still to come.
        let hi = (ideal + window).min(len - (n - k));
        let lo = (previous + 1).max(ideal.saturating_sub(window)).min(hi);
        let cut = (lo..=hi)
            .filter_map(|c| score(c, ideal).map(|s| (c, s)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(c, _)| c)
            .unwrap_or_else(|| ideal.clamp(lo, hi));
        cuts.push(cut);
        previous = cut;
    }
    cuts
}

/// Cuts `0..items.len()` into the fewest balanced ranges that all `fit`, starting from
/// `parts` pieces (or a single unit per range at worst).
fn fitting_ranges(
    items: &[Item],
    mut parts: usize,
    fits: impl Fn(Range<usize>) -> bool,
) -> Vec<Range<usize>> {
    loop {
        let mut bounds = vec![0usize];
        bounds.extend(chunk_cuts(items, parts));
        bounds.push(items.len());
        let ranges: Vec<Range<usize>> = bounds.windows(2).map(|w| w[0]..w[1]).collect();
        if ranges.iter().all(|r| fits(r.clone())) || parts >= items.len() {
            return ranges;
        }
        parts += 1;
    }
}

fn split_if_too_long(seg: &Segment) -> Vec<Segment> {
    let duration = seg.end - seg.start;
    let max_width = column_limit(&seg.text, MAX_CHARS);
    let width = display_width(&seg.text);

    if duration <= MAX_DURATION_SECS && width <= max_width {
        return vec![seg.clone()];
    }

    let units = units_of(&seg.text);
    if units.len() < 2 {
        return vec![seg.clone()];
    }
    let texts: Vec<&str> = units.iter().map(|u| u.text.as_str()).collect();
    let items = items_for(&texts, |_| 0.0);

    let parts = ((duration / MAX_DURATION_SECS).ceil() as usize)
        .max(width.div_ceil(max_width.max(1)))
        .max(2);

    // Balanced pieces can still overshoot a limit when the words are uneven: add a
    // piece until every one of them fits (or each piece is a single unit).
    let ranges = fitting_ranges(&items, parts, |r| {
        let piece = display_width(&render_units(&units[r]));
        let share = duration * piece as f64 / width.max(1) as f64;
        piece <= max_width && share <= MAX_DURATION_SECS
    });

    let pieces: Vec<String> = ranges
        .iter()
        .map(|r| render_units(&units[r.clone()]))
        .collect();
    let total: usize = pieces
        .iter()
        .map(|p| display_width(p))
        .sum::<usize>()
        .max(1);
    let mut consumed = 0usize;
    pieces
        .into_iter()
        .map(|text| {
            let start = seg.start + duration * consumed as f64 / total as f64;
            consumed += display_width(&text);
            let end = seg.start + duration * consumed as f64 / total as f64;
            Segment { start, end, text }
        })
        .collect()
}

/// A word with the time whisper gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Limits a cue must respect: duration in seconds and columns (stated for Latin text).
#[derive(Debug, Clone, Copy)]
pub struct CueLimits {
    pub max_secs: f64,
    pub max_chars: usize,
}

pub const DEFAULT_LIMITS: CueLimits = CueLimits {
    max_secs: MAX_DURATION_SECS,
    max_chars: MAX_CHARS,
};

fn timed_segment(words: &[TimedWord]) -> Segment {
    Segment {
        start: words[0].start,
        end: words[words.len() - 1].end.max(words[0].start),
        text: join_words(words.iter().map(|w| w.text.as_str())),
    }
}

/// Splits a segment into cues using the real time of each word: a cue never spans a long
/// silence, and the other cuts fall where the speech pauses or a sentence ends rather
/// than where a proportional guess puts them. A segment that already respects `limits`
/// is returned whole.
pub fn split_timed(words: &[TimedWord], limits: CueLimits) -> Vec<Segment> {
    if words.is_empty() {
        return Vec::new();
    }
    let mut groups: Vec<&[TimedWord]> = Vec::new();
    let mut from = 0;
    for i in 1..words.len() {
        if words[i].start - words[i - 1].end >= LONG_PAUSE_SECS {
            groups.push(&words[from..i]);
            from = i;
        }
    }
    groups.push(&words[from..]);
    groups
        .into_iter()
        .flat_map(|group| split_group(group, limits))
        .collect()
}

fn split_group(words: &[TimedWord], limits: CueLimits) -> Vec<Segment> {
    let whole = timed_segment(words);
    let max_width = column_limit(&whole.text, limits.max_chars);
    let width = display_width(&whole.text);
    if words.len() < 2 || (whole.end - whole.start <= limits.max_secs && width <= max_width) {
        return vec![whole];
    }

    let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let items = items_for(&texts, |i| {
        words
            .get(i + 1)
            .map_or(0.0, |next| next.start - words[i].end)
    });
    let parts = ((whole.end - whole.start) / limits.max_secs)
        .ceil()
        .max(width.div_ceil(max_width.max(1)) as f64)
        .max(2.0) as usize;
    fitting_ranges(&items, parts, |r| {
        let piece = timed_segment(&words[r]);
        piece.end - piece.start <= limits.max_secs && display_width(&piece.text) <= max_width
    })
    .into_iter()
    .map(|r| timed_segment(&words[r]))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start,
            end,
            text: text.to_string(),
        }
    }

    fn assert_within_limits(cues: &[Segment]) {
        for cue in cues {
            assert!(cue.text.chars().count() <= MAX_CHARS, "too long: {cue:?}");
            assert!(
                cue.end - cue.start <= MAX_DURATION_SECS + 1e-9,
                "too slow: {cue:?}"
            );
        }
    }

    #[test]
    fn short_segment_is_untouched() {
        let segments = vec![seg(0.0, 2.0, "Bonjour tout le monde.")];
        let cues = build_cues(&segments);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Bonjour tout le monde.");
    }

    #[test]
    fn long_duration_segment_is_split() {
        let long_text = "Ceci est une phrase assez longue qui dépasse largement la durée maximale autorisée pour une seule cue de sous-titre et doit donc être coupée en plusieurs morceaux.";
        let segments = vec![seg(0.0, 15.0, long_text)];
        let cues = build_cues(&segments);
        assert!(cues.len() >= 2, "expected several cues, got {}", cues.len());

        // Timestamps must increase and cover the whole original interval.
        assert_eq!(cues.first().unwrap().start, 0.0);
        assert!((cues.last().unwrap().end - 15.0).abs() < 1e-9);
        for pair in cues.windows(2) {
            assert!(pair[0].end <= pair[1].start + 1e-9);
        }
        assert_within_limits(&cues);
    }

    #[test]
    fn long_char_count_segment_is_split() {
        let long_text = "a ".repeat(60); // 120 characters: under the duration threshold but over the character threshold
        let segments = vec![seg(0.0, 3.0, long_text.trim())];
        let cues = build_cues(&segments);
        assert!(cues.len() >= 2);
        assert_within_limits(&cues);
    }

    #[test]
    fn single_word_segment_is_never_split() {
        let segments = vec![seg(0.0, 10.0, "Voila")];
        let cues = build_cues(&segments);
        assert_eq!(cues.len(), 1);
    }

    #[test]
    fn the_pieces_respect_the_character_limit_even_with_long_words() {
        // 20 words of 8 letters = 179 characters: two pieces of 89 would break the limit.
        let text = ["abcdefgh"; 20].join(" ");

        let cues = build_cues(&[seg(0.0, 3.0, &text)]);

        assert_eq!(cues.len(), 3);
        assert_within_limits(&cues);
    }

    #[test]
    fn the_pieces_are_balanced() {
        let text = ["word"; 10].join(" ");

        let cues = build_cues(&[seg(0.0, 10.0, &text)]);

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text.split(' ').count(), 5);
        assert!((cues[0].end - 5.0).abs() < 1e-9);
        assert_within_limits(&cues);
    }

    #[test]
    fn thirty_words_over_fifteen_seconds_give_three_even_pieces() {
        let text = (0..30)
            .map(|i| format!("w{i:02}"))
            .collect::<Vec<_>>()
            .join(" ");

        let cues = build_cues(&[seg(0.0, 15.0, &text)]);

        assert_eq!(cues.len(), 3);
        for cue in &cues {
            assert_eq!(cue.text.split(' ').count(), 10, "{cue:?}");
        }
        assert_within_limits(&cues);
    }

    #[test]
    fn a_cut_slides_to_a_nearby_sentence_end() {
        let text = "One two three four five. Six seven eight nine ten eleven twelve thirteen.";

        let cues = build_cues(&[seg(0.0, 9.0, text)]);

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "One two three four five.");
        assert!(cues[1].text.starts_with("Six"));
    }

    #[test]
    fn a_sentence_end_just_before_the_middle_is_found_too() {
        // The break sits one word *before* the ideal cut: the old code never looked back.
        let text = "aaa bbb ccc ddd eee fff ggg. hhh iii jjj kkk lll mmm nnn ooo ppp";

        let cues = build_cues(&[seg(0.0, 8.0, text)]);

        assert_eq!(cues[0].text, "aaa bbb ccc ddd eee fff ggg.");
    }

    #[test]
    fn empty_segments_are_dropped() {
        let cues = build_cues(&[
            seg(0.0, 1.0, "Hello"),
            seg(1.0, 2.0, "   "),
            seg(2.0, 3.0, ""),
            seg(3.0, 4.0, "World"),
        ]);

        let texts: Vec<_> = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello", "World"]);
    }

    #[test]
    fn a_segment_ending_before_it_starts_is_repaired() {
        let cues = build_cues(&[seg(5.0, 4.0, "Odd")]);

        assert!(cues[0].end >= cues[0].start);
    }

    #[test]
    fn overlapping_cues_are_pulled_apart() {
        let cues = build_cues(&[seg(0.0, 3.0, "First"), seg(2.0, 5.0, "Second")]);

        assert!(cues[0].end <= cues[1].start + 1e-9);
        assert_eq!(cues[1].start, 2.0);
    }

    #[test]
    fn a_very_short_cue_is_stretched_when_there_is_room() {
        let cues = build_cues(&[seg(0.0, 0.1, "Hm"), seg(5.0, 6.0, "Later")]);

        assert!((cues[0].end - cues[0].start - MIN_DURATION_SECS).abs() < 1e-9);
    }

    #[test]
    fn a_short_cue_never_runs_into_the_next_one() {
        let cues = build_cues(&[seg(0.0, 0.1, "Hm"), seg(0.2, 2.0, "Next")]);

        assert!(cues[0].end <= cues[1].start + 1e-9);
    }

    #[test]
    fn every_word_survives_the_split_in_order() {
        let text = (0..57)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");

        let cues = build_cues(&[seg(0.0, 40.0, &text)]);

        let rejoined = cues
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, text);
        assert_within_limits(&cues);
    }

    fn word(text: &str, start: f64, end: f64) -> TimedWord {
        TimedWord {
            text: text.into(),
            start,
            end,
        }
    }

    #[test]
    fn a_short_timed_segment_stays_whole() {
        let words = [word("Hello", 1.0, 1.4), word("world", 1.5, 2.0)];

        let cues = split_timed(&words, DEFAULT_LIMITS);

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Hello world");
        assert_eq!((cues[0].start, cues[0].end), (1.0, 2.0));
    }

    #[test]
    fn a_long_timed_segment_is_cut_at_real_word_times() {
        // 12 words, one per second, with a long pause after the 6th: the cut must fall
        // there and the second cue must start when its first word is actually spoken.
        let mut words = Vec::new();
        for i in 0..12 {
            let start = i as f64 + if i >= 6 { 3.0 } else { 0.0 };
            words.push(word(&format!("w{i}"), start, start + 0.8));
        }

        let cues = split_timed(
            &words,
            CueLimits {
                max_secs: 8.0,
                max_chars: 84,
            },
        );

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "w0 w1 w2 w3 w4 w5");
        assert_eq!(cues[1].start, 9.0);
        assert_eq!(cues[1].end, 14.8);
    }

    #[test]
    fn every_timed_piece_respects_the_limits_and_keeps_all_words() {
        let words: Vec<TimedWord> = (0..40)
            .map(|i| word(&format!("word{i}"), i as f64 * 0.7, i as f64 * 0.7 + 0.6))
            .collect();

        let cues = split_timed(&words, DEFAULT_LIMITS);

        for cue in &cues {
            assert!(cue.end - cue.start <= 7.0 + 1e-9, "{cue:?}");
            assert!(cue.text.chars().count() <= 84, "{cue:?}");
        }
        let rejoined = cues
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let original = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, original);
        assert!(cues.windows(2).all(|p| p[0].end <= p[1].start + 1e-9));
    }

    #[test]
    fn a_timed_cut_prefers_a_sentence_end() {
        let words = [
            word("One", 0.0, 0.5),
            word("two", 0.6, 1.0),
            word("three.", 1.1, 1.6),
            word("Four", 1.7, 2.2),
            word("five", 2.3, 2.8),
            word("six", 2.9, 3.4),
        ];

        let cues = split_timed(
            &words,
            CueLimits {
                max_secs: 2.0,
                max_chars: 84,
            },
        );

        assert_eq!(cues[0].text, "One two three.");
    }

    #[test]
    fn nothing_to_split_yields_nothing() {
        assert!(split_timed(&[], DEFAULT_LIMITS).is_empty());
    }

    #[test]
    fn a_long_silence_ends_the_cue_even_when_the_segment_is_short() {
        // The cue used to stay on screen through the 4 s pause after the third word.
        let mut words = Vec::new();
        for i in 0..12 {
            let start = i as f64 + if i >= 3 { 4.0 } else { 0.0 };
            words.push(word(&format!("w{i}"), start, start + 0.8));
        }

        let cues = split_timed(
            &words,
            CueLimits {
                max_secs: 8.0,
                max_chars: 84,
            },
        );

        assert_eq!(cues[0].text, "w0 w1 w2");
        assert_eq!((cues[0].start, cues[0].end), (0.0, 2.8));
        assert_eq!(cues[1].start, 7.0);
    }

    #[test]
    fn a_shorter_pause_attracts_the_cut() {
        // 10 words, one second each, with a 1 s gap after the 4th: ideal cut is at 5.
        let words: Vec<TimedWord> = (0..10)
            .map(|i| {
                let start = i as f64 * 0.7 + if i >= 4 { 1.0 } else { 0.0 };
                word(&format!("w{i}"), start, start + 0.6)
            })
            .collect();

        let cues = split_timed(
            &words,
            CueLimits {
                max_secs: 4.5,
                max_chars: 84,
            },
        );

        assert_eq!(cues[0].text, "w0 w1 w2 w3");
    }

    fn chinese() -> String {
        "我们今天在北京开会讨论经济发展和环境保护的问题大家都同意必须尽快采取行动这部电影讲述了一个年轻人离开家乡寻找梦想的故事".to_string()
    }

    #[test]
    fn a_chinese_segment_is_split_without_any_space() {
        let text = chinese();

        let cues = build_cues(&[seg(0.0, 20.0, &text)]);

        assert!(cues.len() >= 2, "{cues:?}");
        assert!(
            cues.iter().all(|c| display_width(&c.text) <= 64),
            "{cues:?}"
        );
        assert!(cues
            .iter()
            .all(|c| c.end - c.start <= MAX_DURATION_SECS + 1e-9));
        let rejoined: String = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(rejoined, text);
    }

    #[test]
    fn full_width_characters_take_two_columns() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("你好"), 4);
        assert_eq!(line_limit("你好"), WIDE_LINE_COLUMNS);
        assert_eq!(line_limit("hello"), LATIN_LINE_COLUMNS);
    }

    #[test]
    fn timed_chinese_words_are_joined_without_spaces() {
        let words = [
            word("我们", 0.0, 0.5),
            word("今天", 0.5, 1.0),
            word("ok", 1.0, 1.4),
        ];

        let cues = split_timed(&words, DEFAULT_LIMITS);

        assert_eq!(cues[0].text, "我们今天ok");
    }

    #[test]
    fn units_remember_the_spaces_of_the_original() {
        let units = units_of("Hello 世界 ok");

        assert_eq!(render_units(&units), "Hello 世界 ok");
        assert_eq!(units.len(), 4);
        assert!(!units[2].space_before);
    }

    #[test]
    fn cues_are_put_in_order() {
        let cues = build_cues(&[seg(5.0, 6.0, "B"), seg(1.0, 2.0, "A")]);

        assert_eq!(cues[0].text, "A");
        assert!(cues.iter().all(|c| c.end > c.start));
    }

    #[test]
    fn a_dense_cue_gets_time_to_be_read_when_there_is_room() {
        let text = "x".repeat(60);

        let cues = build_cues(&[seg(0.0, 1.0, &text), seg(30.0, 31.0, "later")]);

        assert!(cues[0].end - cues[0].start >= 60.0 / MAX_READING_SPEED - 1e-9);
        assert!(cues[0].end <= 30.0 - MIN_GAP_SECS + 1e-9);
    }

    #[test]
    fn lines_of_a_cue_flatten_to_one() {
        assert_eq!(flatten_lines("- Hi!\n- Hello."), "- Hi! - Hello.");
        assert_eq!(flatten_lines("你好\n世界"), "你好世界");
    }
}
