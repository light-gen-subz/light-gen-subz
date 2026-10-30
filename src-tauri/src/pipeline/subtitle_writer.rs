use crate::pipeline::segmentation::{display_width, line_limit, render_units, units_of, Unit};
use crate::pipeline::stt::Segment;

/// Parses SRT text back into cues (inverse of [`to_srt`]); the lines of a cue are kept,
/// separated by `\n`. Tolerant of CRLF, a BOM, extra blank lines and cues without a
/// sequence number.
pub fn parse_srt(content: &str) -> Vec<Segment> {
    let normalized = content
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");

    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in normalized.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }

    blocks.into_iter().filter_map(parse_block).collect()
}

fn parse_block(lines: Vec<&str>) -> Option<Segment> {
    // The sequence number is optional in the wild: find the timing line.
    let timing_index = lines.iter().position(|l| l.contains(" --> "))?;
    let (start, end) = lines[timing_index].split_once(" --> ")?;
    let text = lines[timing_index + 1..]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    Some(Segment {
        start: parse_timestamp(start)?,
        end: parse_timestamp(end)?,
        text,
    })
}

fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    let (hms, ms) = s.split_once([',', '.'])?;
    let mut parts = hms.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let mins: f64 = parts.next()?.parse().ok()?;
    let secs: f64 = parts.next()?.parse().ok()?;
    let millis: f64 = ms.parse().ok()?;
    Some(hours * 3600.0 + mins * 60.0 + secs + millis / 1000.0)
}

/// Wraps `text` on as few lines as possible, keeping them about the same length. A line
/// break the text already has is kept (dialogue, hand-made edits); only a line that is too
/// long is wrapped.
pub fn wrap_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(wrap_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cost of a way of cutting a line, best first: width over the limit, breaks before a
/// closing punctuation mark, widest line, then how uneven the lines are.
type Cost = (usize, usize, usize, usize);

fn wrap_line(line: &str) -> String {
    let units = units_of(line);
    let limit = line_limit(line);
    if display_width(&units_text(&units)) <= limit || units.len() < 2 {
        return units_text(&units);
    }
    // width[a][b]: columns of units a..b on one line
    let n_units = units.len();
    let width = |a: usize, b: usize| display_width(&render_units(&units[a..b]));
    let bad_break = |at: usize| {
        usize::from(
            units[at]
                .text
                .chars()
                .next()
                .is_some_and(|c| "。、，．？！；：）」』】〉》”’,.;:?!)]}…".contains(c)),
        )
    };
    let start_lines = display_width(&units_text(&units)).div_ceil(limit).max(2);
    let mut best: Option<Vec<usize>> = None;
    for lines in start_lines..=n_units {
        let mut cost: Vec<Vec<Option<Cost>>> = vec![vec![None; n_units + 1]; lines + 1];
        let mut from = vec![vec![0usize; n_units + 1]; lines + 1];
        cost[0][0] = Some((0, 0, 0, 0));
        for k in 1..=lines {
            for i in k..=n_units {
                for j in (k - 1)..i {
                    let Some(prev) = cost[k - 1][j] else { continue };
                    let w = width(j, i);
                    let candidate = (
                        prev.0.max(w.saturating_sub(limit)),
                        prev.1 + if j > 0 { bad_break(j) } else { 0 },
                        prev.2.max(w),
                        prev.3 + w * w,
                    );
                    if cost[k][i].is_none_or(|c| candidate < c) {
                        cost[k][i] = Some(candidate);
                        from[k][i] = j;
                    }
                }
            }
        }
        let Some(done) = cost[lines][n_units] else {
            continue;
        };
        let mut cuts = Vec::new();
        let mut at = n_units;
        for k in (1..=lines).rev() {
            at = from[k][at];
            cuts.push(at);
        }
        cuts.reverse();
        best = Some(cuts);
        if done.0 == 0 {
            break;
        }
    }
    let cuts = best.unwrap_or_else(|| vec![0]);
    let mut out = Vec::new();
    for (i, &from) in cuts.iter().enumerate() {
        let to = cuts.get(i + 1).copied().unwrap_or(n_units);
        out.push(render_units(&units[from..to]));
    }
    out.join("\n")
}

fn units_text(units: &[Unit]) -> String {
    render_units(units)
}

/// Serialises a list of cues as SRT text. Cues without text are skipped: an empty cue
/// would produce two consecutive line breaks and corrupt how readers split the file.
pub fn to_srt(cues: &[Segment]) -> String {
    render(cues, ',', "", |t| t.to_string())
}

/// WebVTT reads `<` as a tag, `&` as an entity and forbids `-->` in a cue's text.
fn escape_vtt(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Same cues as WebVTT.
pub fn to_vtt(cues: &[Segment]) -> String {
    render(cues, '.', "WEBVTT\n\n", escape_vtt)
}

/// Same cues as an Advanced SubStation Alpha script (default style, no effects).
pub fn to_ass(cues: &[Segment]) -> String {
    let mut out = String::from(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\nWrapStyle: 0\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,Arial,64,&H00FFFFFF,&H000000FF,&H00000000,&H64000000,0,0,0,0,100,100,0,0,1,3,1,2,60,60,50,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for cue in cues {
        let text = wrap_text(&cue.text);
        if text.is_empty() {
            continue;
        }
        // `{` starts an override block in ASS, a newline is `\N`, and a backslash would
        // start an escape: a zero-width space after it keeps it literal.
        let text = text
            .replace('\\', "\\\u{200b}")
            .replace('{', "(")
            .replace('}', ")")
            .replace('\n', "\\N");
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{text}\n",
            ass_timestamp(cue.start),
            ass_timestamp(cue.end)
        ));
    }
    out
}

/// `H:MM:SS.cc` (centiseconds), the only precision ASS has.
fn ass_timestamp(seconds: f64) -> String {
    let total_cs = (seconds.max(0.0) * 100.0).round() as u64;
    let cs = total_cs % 100;
    let total_secs = total_cs / 100;
    format!(
        "{}:{:02}:{:02}.{cs:02}",
        total_secs / 3600,
        (total_secs / 60) % 60,
        total_secs % 60
    )
}

/// Puts edited texts back onto the cues, keeping their timings. The number of texts must
/// match: an edit never adds or removes a cue.
pub fn apply_text_edits(cues: &[Segment], texts: Vec<String>) -> anyhow::Result<Vec<Segment>> {
    anyhow::ensure!(
        texts.len() == cues.len(),
        "{} edited texts for {} cues",
        texts.len(),
        cues.len()
    );
    Ok(cues
        .iter()
        .zip(texts)
        .map(|(cue, text)| Segment {
            start: cue.start,
            end: cue.end,
            text,
        })
        .collect())
}

fn render(cues: &[Segment], decimal: char, header: &str, escape: fn(&str) -> String) -> String {
    let mut out = String::from(header);
    let mut number = 0;
    for cue in cues {
        let text = wrap_text(&cue.text);
        if text.is_empty() {
            continue;
        }
        number += 1;
        out.push_str(&format!(
            "{number}\n{} --> {}\n{}\n\n",
            format_timestamp(cue.start, decimal),
            format_timestamp(cue.end, decimal),
            escape(&text)
        ));
    }
    out
}

/// Formats a duration in seconds as `HH:MM:SS,mmm` (or `.mmm` for WebVTT).
fn format_timestamp(seconds: f64, decimal: char) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_secs = total_ms / 1000;
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;
    format!("{hours:02}:{mins:02}:{secs:02}{decimal}{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamp_correctly() {
        assert_eq!(format_timestamp(0.0, ','), "00:00:00,000");
        assert_eq!(format_timestamp(61.234, ','), "00:01:01,234");
        assert_eq!(format_timestamp(3661.5, ','), "01:01:01,500");
    }

    #[test]
    fn writes_srt_with_sequential_indices() {
        let cues = vec![
            Segment {
                start: 0.0,
                end: 1.5,
                text: "Bonjour.".into(),
            },
            Segment {
                start: 1.5,
                end: 3.0,
                text: "Comment ça va ?".into(),
            },
        ];
        let srt = to_srt(&cues);
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:01,500\nBonjour.\n\n"));
        assert!(srt.contains("2\n00:00:01,500 --> 00:00:03,000\nComment ça va ?\n\n"));
    }

    #[test]
    fn parse_srt_round_trips() {
        let cues = vec![
            Segment {
                start: 0.0,
                end: 1.5,
                text: "Bonjour.".into(),
            },
            Segment {
                start: 1.5,
                end: 3.25,
                text: "Comment ça va ?".into(),
            },
        ];
        let srt = to_srt(&cues);
        let parsed = parse_srt(&srt);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "Bonjour.");
        assert!((parsed[0].start - 0.0).abs() < 1e-6);
        assert!((parsed[1].end - 3.25).abs() < 1e-6);
    }

    fn cue(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start,
            end,
            text: text.into(),
        }
    }

    #[test]
    fn an_empty_cue_is_skipped_and_numbering_stays_continuous() {
        let srt = to_srt(&[
            cue(0.0, 1.0, "Hello"),
            cue(1.0, 2.0, "   "),
            cue(2.0, 3.0, "World"),
        ]);

        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:01,000\nHello\n\n2\n00:00:02,000 --> 00:00:03,000\nWorld\n\n"
        );
    }

    #[test]
    fn what_to_srt_writes_parse_srt_reads_back_whatever_the_cues() {
        let cues = [
            cue(0.0, 1.0, "Hello"),
            cue(1.0, 2.0, ""),
            cue(2.0, 3.0, "World"),
        ];

        let parsed = parse_srt(&to_srt(&cues));

        let texts: Vec<_> = parsed.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello", "World"]);
    }

    #[test]
    fn extra_blank_lines_between_cues_are_tolerated() {
        let parsed = parse_srt(
            "1\n00:00:00,000 --> 00:00:01,000\nA\n\n\n\n2\n00:00:01,000 --> 00:00:02,000\nB\n",
        );

        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn crlf_and_a_bom_are_tolerated() {
        let parsed = parse_srt("\u{feff}1\r\n00:00:00,000 --> 00:00:01,000\r\nA\r\n\r\n2\r\n00:00:01,000 --> 00:00:02,000\r\nB\r\n");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "A");
    }

    #[test]
    fn a_cue_without_a_sequence_number_is_read() {
        let parsed = parse_srt("00:00:00,000 --> 00:00:01,000\nA\n");

        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn the_lines_of_a_cue_are_kept() {
        let parsed = parse_srt("1\n00:00:00,000 --> 00:00:01,000\nfirst\nsecond\n");

        assert_eq!(parsed[0].text, "first\nsecond");
    }

    #[test]
    fn a_short_text_stays_on_one_line() {
        assert_eq!(wrap_text("Comment ça va ?"), "Comment ça va ?");
    }

    #[test]
    fn a_long_text_is_wrapped_into_balanced_lines() {
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen";

        let wrapped = wrap_text(text);

        let lines: Vec<_> = wrapped.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(line.chars().count() <= 42, "{line}");
        }
        assert!(lines[0].chars().count().abs_diff(lines[1].chars().count()) < 14);
        assert_eq!(wrapped.replace('\n', " "), text);
    }

    #[test]
    fn wrapping_never_loses_a_word() {
        let text = (0..40)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");

        let wrapped = wrap_text(&text);

        assert_eq!(wrapped.split_whitespace().count(), 40);
        assert!(wrapped.lines().all(|l| l.chars().count() <= 42));
    }

    #[test]
    fn a_word_longer_than_a_line_is_left_whole() {
        let word = "x".repeat(60);

        assert_eq!(wrap_text(&word), word);
    }

    #[test]
    fn webvtt_has_a_header_and_dotted_timestamps() {
        let vtt = to_vtt(&[cue(0.0, 1.5, "Hi")]);

        assert_eq!(vtt, "WEBVTT\n\n1\n00:00:00.000 --> 00:00:01.500\nHi\n\n");
    }

    #[test]
    fn dotted_timestamps_are_also_parsed() {
        let parsed = parse_srt("1\n00:00:01.500 --> 00:00:02.000\nA\n");

        assert!((parsed[0].start - 1.5).abs() < 1e-9);
    }

    #[test]
    fn an_ass_script_has_its_three_sections_and_one_dialogue_per_cue() {
        let ass = to_ass(&[cue(0.0, 1.5, "Hi"), cue(61.25, 3725.0, "Bye")]);

        assert!(
            ass.contains("[Script Info]")
                && ass.contains("[V4+ Styles]")
                && ass.contains("[Events]")
        );
        assert!(ass.contains("Dialogue: 0,0:00:00.00,0:00:01.50,Default,,0,0,0,,Hi"));
        assert!(ass.contains("Dialogue: 0,0:01:01.25,1:02:05.00,Default,,0,0,0,,Bye"));
    }

    #[test]
    fn an_ass_line_break_is_backslash_n_and_braces_cannot_open_an_override() {
        let long = "one two three four five six seven eight nine ten eleven twelve {x}";

        let ass = to_ass(&[cue(0.0, 1.0, long)]);

        assert!(ass.contains("\\N"));
        assert!(!ass.contains("{x}"));
    }

    #[test]
    fn an_empty_cue_is_not_written_to_the_ass_script() {
        assert_eq!(to_ass(&[cue(0.0, 1.0, " ")]).matches("Dialogue").count(), 0);
    }

    #[test]
    fn edits_replace_the_texts_and_keep_the_timings() {
        let cues = [cue(0.0, 1.0, "a"), cue(1.0, 2.0, "b")];

        let out = apply_text_edits(&cues, vec!["A".into(), "B2".into()]).unwrap();

        assert_eq!(out[1].text, "B2");
        assert_eq!(out[1].start, 1.0);
        assert_eq!(out[1].end, 2.0);
    }

    #[test]
    fn edits_that_add_or_drop_a_cue_are_refused() {
        let cues = [cue(0.0, 1.0, "a"), cue(1.0, 2.0, "b")];

        assert!(apply_text_edits(&cues, vec!["A".into()]).is_err());
        assert!(apply_text_edits(&cues, vec!["A".into(), "B".into(), "C".into()]).is_err());
    }

    #[test]
    fn a_hand_made_line_break_survives_writing_and_reading() {
        let edited =
            apply_text_edits(&[cue(0.0, 2.0, "x")], vec!["- Hi!\n- Hello.".into()]).unwrap();

        let srt = to_srt(&edited);

        assert!(srt.contains("- Hi!\n- Hello.\n\n"), "{srt}");
        assert_eq!(parse_srt(&srt)[0].text, "- Hi!\n- Hello.");
    }

    #[test]
    fn no_line_goes_over_the_limit_when_it_can_be_avoided() {
        // 85 columns used to give a 44-column line.
        let text = "abcd ".repeat(17);

        let wrapped = wrap_text(text.trim());

        assert!(
            wrapped.lines().all(|l| l.chars().count() <= 42),
            "{wrapped}"
        );
        assert_eq!(wrapped.split_whitespace().count(), 17);
    }

    #[test]
    fn a_text_without_spaces_is_cut_between_characters() {
        let text = "我们今天在北京开会讨论经济发展和环境保护的问题大家都同意必须尽快采取行动";

        let wrapped = wrap_text(text);

        assert!(wrapped.lines().count() >= 2);
        assert!(wrapped.lines().all(|l| display_width(l) <= 32), "{wrapped}");
        assert_eq!(wrapped.replace('\n', ""), text);
    }

    #[test]
    fn a_closing_punctuation_mark_never_starts_a_line() {
        let text = "我们今天在北京开会讨论经济发展和环境保护的问题，大家都同意必须尽快采取行动。";

        let wrapped = wrap_text(text);

        assert!(
            wrapped.lines().all(|l| !l.starts_with(['，', '。'])),
            "{wrapped}"
        );
    }

    #[test]
    fn webvtt_text_is_escaped() {
        let vtt = to_vtt(&[cue(0.0, 1.0, "a < b && c --> d")]);

        assert!(vtt.contains("a &lt; b &amp;&amp; c --&gt; d"), "{vtt}");
    }

    #[test]
    fn an_ass_backslash_stays_literal() {
        let ass = to_ass(&[cue(0.0, 1.0, "path\\Nname")]);

        assert!(ass.contains("path\\\u{200b}Nname"), "{ass}");
    }
}
