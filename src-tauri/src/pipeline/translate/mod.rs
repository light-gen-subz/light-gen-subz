pub mod azure_translate;
pub mod cloud_deepl;
pub mod google_translate;
pub mod languages;
pub mod local_nllb;
pub mod nllb_models;
pub mod openai_translate;

pub use azure_translate::AzureTranslateEngine;
pub use cloud_deepl::CloudDeepLEngine;
pub use google_translate::GoogleTranslateEngine;
pub use local_nllb::LocalNllbEngine;
pub use openai_translate::OpenAiTranslateEngine;

pub trait TranslationEngine {
    /// Translates a batch of texts, in order. `source_lang` is `None` for auto-detect.
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> anyhow::Result<Vec<String>>;
}
