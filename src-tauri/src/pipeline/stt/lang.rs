/// Whisper's language names (as returned by `verbose_json`) and their ISO-639 codes.
const WHISPER_LANGUAGES: &[(&str, &str)] = &[
    ("english", "en"),
    ("chinese", "zh"),
    ("german", "de"),
    ("spanish", "es"),
    ("russian", "ru"),
    ("korean", "ko"),
    ("french", "fr"),
    ("japanese", "ja"),
    ("portuguese", "pt"),
    ("turkish", "tr"),
    ("polish", "pl"),
    ("catalan", "ca"),
    ("dutch", "nl"),
    ("arabic", "ar"),
    ("swedish", "sv"),
    ("italian", "it"),
    ("indonesian", "id"),
    ("hindi", "hi"),
    ("finnish", "fi"),
    ("vietnamese", "vi"),
    ("hebrew", "he"),
    ("ukrainian", "uk"),
    ("greek", "el"),
    ("malay", "ms"),
    ("czech", "cs"),
    ("romanian", "ro"),
    ("danish", "da"),
    ("hungarian", "hu"),
    ("tamil", "ta"),
    ("norwegian", "no"),
    ("thai", "th"),
    ("urdu", "ur"),
    ("croatian", "hr"),
    ("bulgarian", "bg"),
    ("lithuanian", "lt"),
    ("latin", "la"),
    ("maori", "mi"),
    ("malayalam", "ml"),
    ("welsh", "cy"),
    ("slovak", "sk"),
    ("telugu", "te"),
    ("persian", "fa"),
    ("latvian", "lv"),
    ("bengali", "bn"),
    ("serbian", "sr"),
    ("azerbaijani", "az"),
    ("slovenian", "sl"),
    ("kannada", "kn"),
    ("estonian", "et"),
    ("macedonian", "mk"),
    ("breton", "br"),
    ("basque", "eu"),
    ("icelandic", "is"),
    ("armenian", "hy"),
    ("nepali", "ne"),
    ("mongolian", "mn"),
    ("bosnian", "bs"),
    ("kazakh", "kk"),
    ("albanian", "sq"),
    ("swahili", "sw"),
    ("galician", "gl"),
    ("marathi", "mr"),
    ("punjabi", "pa"),
    ("sinhala", "si"),
    ("khmer", "km"),
    ("shona", "sn"),
    ("yoruba", "yo"),
    ("somali", "so"),
    ("afrikaans", "af"),
    ("occitan", "oc"),
    ("georgian", "ka"),
    ("belarusian", "be"),
    ("tajik", "tg"),
    ("sindhi", "sd"),
    ("gujarati", "gu"),
    ("amharic", "am"),
    ("yiddish", "yi"),
    ("lao", "lo"),
    ("uzbek", "uz"),
    ("faroese", "fo"),
    ("haitian creole", "ht"),
    ("pashto", "ps"),
    ("turkmen", "tk"),
    ("nynorsk", "nn"),
    ("maltese", "mt"),
    ("sanskrit", "sa"),
    ("luxembourgish", "lb"),
    ("myanmar", "my"),
    ("tibetan", "bo"),
    ("tagalog", "tl"),
    ("malagasy", "mg"),
    ("assamese", "as"),
    ("tatar", "tt"),
    ("hawaiian", "haw"),
    ("lingala", "ln"),
    ("hausa", "ha"),
    ("bashkir", "ba"),
    ("javanese", "jw"),
    ("sundanese", "su"),
    ("cantonese", "yue"),
];

/// A language reported by a transcription API, as a lower-case ISO code.
///
/// OpenAI and Groq answer with the English *name* (`"german"`), whereas other engines
/// answer with a code (`"de"`, `"en-US"`). Anything unrecognised becomes `"unknown"`
/// rather than a made-up code.
pub fn to_code(reported: &str) -> String {
    let lower = reported.trim().to_ascii_lowercase();
    if let Some((_, code)) = WHISPER_LANGUAGES.iter().find(|(name, _)| *name == lower) {
        return (*code).to_string();
    }
    let primary = lower.split(['-', '_']).next().unwrap_or("");
    if (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_lowercase()) {
        return primary.to_string();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::translate::languages::LANGUAGES;

    #[test]
    fn a_language_name_becomes_its_iso_code() {
        assert_eq!(to_code("french"), "fr");
        assert_eq!(to_code("German"), "de");
    }

    #[test]
    fn the_names_whose_first_letters_are_not_the_code_are_right() {
        // The first two letters of these names are not their code — or worse, they are
        // the code of another language (swedish → "sw" is Swahili).
        assert_eq!(to_code("spanish"), "es");
        assert_eq!(to_code("swedish"), "sv");
        assert_eq!(to_code("chinese"), "zh");
        assert_eq!(to_code("greek"), "el");
        assert_eq!(to_code("czech"), "cs");
        assert_eq!(to_code("polish"), "pl");
        assert_eq!(to_code("indonesian"), "id");
    }

    #[test]
    fn every_language_of_the_ui_is_reachable_from_its_whisper_name() {
        for lang in LANGUAGES {
            assert!(
                WHISPER_LANGUAGES.iter().any(|(_, code)| *code == lang.code),
                "{} has no whisper name",
                lang.code
            );
        }
    }

    #[test]
    fn a_code_is_kept_and_a_region_is_dropped() {
        assert_eq!(to_code("fr"), "fr");
        assert_eq!(to_code("en-US"), "en");
        assert_eq!(to_code("PT_br"), "pt");
    }

    #[test]
    fn something_unrecognisable_is_unknown() {
        assert_eq!(to_code(""), "unknown");
        assert_eq!(to_code("klingonese"), "unknown");
        assert_eq!(to_code("12"), "unknown");
    }
}
