use crate::pipeline::stt::Segment;

const MAX_DURATION_SECS: f64 = 7.0;
const MAX_CHARS: usize = 84;

/// Transforme les segments bruts du moteur STT en cues de sous-titres lisibles.
/// MVP : découpe uniquement les segments trop longs (durée ou nombre de caractères),
/// en répartissant le temps proportionnellement au nombre de mots (pas de timestamps par mot).
pub fn build_cues(segments: &[Segment]) -> Vec<Segment> {
    segments.iter().flat_map(split_if_too_long).collect()
}

fn split_if_too_long(seg: &Segment) -> Vec<Segment> {
    let duration = seg.end - seg.start;
    let char_count = seg.text.chars().count();

    if duration <= MAX_DURATION_SECS && char_count <= MAX_CHARS {
        return vec![seg.clone()];
    }

    let words: Vec<&str> = seg.text.split_whitespace().collect();
    if words.len() < 2 {
        return vec![seg.clone()];
    }

    let parts_needed = ((duration / MAX_DURATION_SECS).ceil() as usize)
        .max((char_count as f64 / MAX_CHARS as f64).ceil() as usize)
        .max(2);

    let chunks = chunk_words(&words, parts_needed);

    let total_words = words.len() as f64;
    let mut cues = Vec::with_capacity(chunks.len());
    let mut words_consumed = 0f64;
    for chunk in chunks {
        let chunk_word_count = chunk.split_whitespace().count() as f64;
        let start = seg.start + duration * (words_consumed / total_words);
        words_consumed += chunk_word_count;
        let end = seg.start + duration * (words_consumed / total_words);
        cues.push(Segment {
            start,
            end,
            text: chunk,
        });
    }
    cues
}

/// Répartit les mots en `n` groupes à peu près égaux, en coupant de préférence
/// juste après une ponctuation de fin de phrase la plus proche du point de coupe cible.
fn chunk_words(words: &[&str], n: usize) -> Vec<String> {
    let target_size = (words.len() as f64 / n as f64).ceil() as usize;
    let mut chunks = Vec::new();
    let mut current = Vec::new();

    for (i, word) in words.iter().enumerate() {
        current.push(*word);
        let at_boundary = current.len() >= target_size;
        let ends_sentence = word.ends_with(['.', '?', '!']);
        let is_last = i == words.len() - 1;
        if is_last || (at_boundary && (ends_sentence || current.len() >= target_size + 3)) {
            chunks.push(current.join(" "));
            current = Vec::new();
        }
    }
    if !current.is_empty() {
        chunks.push(current.join(" "));
    }
    chunks
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
        assert!(
            cues.len() >= 2,
            "attendu plusieurs cues, obtenu {}",
            cues.len()
        );

        // Les timestamps doivent être croissants et couvrir tout l'intervalle d'origine.
        assert_eq!(cues.first().unwrap().start, 0.0);
        assert!((cues.last().unwrap().end - 15.0).abs() < 1e-9);
        for pair in cues.windows(2) {
            assert!(pair[0].end <= pair[1].start + 1e-9);
        }
    }

    #[test]
    fn long_char_count_segment_is_split() {
        let long_text = "a ".repeat(60); // 120 caractères, sous le seuil de durée mais au-dessus du seuil de caractères
        let segments = vec![seg(0.0, 3.0, long_text.trim())];
        let cues = build_cues(&segments);
        assert!(cues.len() >= 2);
    }

    #[test]
    fn single_word_segment_is_never_split() {
        let segments = vec![seg(0.0, 10.0, "Voila")];
        let cues = build_cues(&segments);
        assert_eq!(cues.len(), 1);
    }
}
