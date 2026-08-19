use crate::pipeline::stt::Segment;

/// Parses SRT text back into cues (inverse of [`to_srt`]).
pub fn parse_srt(content: &str) -> Vec<Segment> {
    content
        .replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|block| {
            let mut lines = block.lines();
            lines.next()?; // sequence number, unused
            let timing = lines.next()?;
            let (start, end) = timing.split_once(" --> ")?;
            let text = lines.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return None;
            }
            Some(Segment {
                start: parse_timestamp(start)?,
                end: parse_timestamp(end)?,
                text,
            })
        })
        .collect()
}

fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    let (hms, ms) = s.split_once(',')?;
    let mut parts = hms.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let mins: f64 = parts.next()?.parse().ok()?;
    let secs: f64 = parts.next()?.parse().ok()?;
    let millis: f64 = ms.parse().ok()?;
    Some(hours * 3600.0 + mins * 60.0 + secs + millis / 1000.0)
}

/// Sérialise une liste de cues en texte au format SRT.
pub fn to_srt(cues: &[Segment]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            format_timestamp(cue.start),
            format_timestamp(cue.end)
        ));
        out.push_str(cue.text.trim());
        out.push_str("\n\n");
    }
    out
}

/// Formate une durée en secondes au format SRT `HH:MM:SS,mmm`.
fn format_timestamp(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_secs = total_ms / 1000;
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;
    format!("{hours:02}:{mins:02}:{secs:02},{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamp_correctly() {
        assert_eq!(format_timestamp(0.0), "00:00:00,000");
        assert_eq!(format_timestamp(61.234), "00:01:01,234");
        assert_eq!(format_timestamp(3661.5), "01:01:01,500");
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
}
