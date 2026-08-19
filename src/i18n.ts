export type Lang = "en" | "fr";

const dict: Record<Lang, Record<string, string>> = {
  en: {
    // Header
    "status.idle": "Idle",
    "status.running": "Running",
    "status.done": "Done",
    "status.error": "Error",
    "header.settings": "Settings",

    // Settings
    "settings.stt": "Transcription engine",
    "settings.translation": "Translation",
    "settings.azureRegion": "Azure region",
    "settings.language": "Language",
    "settings.updates": "Updates",
    "settings.version": "Version",
    "settings.apiKey": "API key",
    "settings.apiKeySaved": "saved",
    "settings.save": "Save",

    // Engines
    "stt.local": "Local (offline, whisper.cpp)",
    "stt.groq": "Groq (cloud)",
    "stt.openai": "OpenAI (cloud)",
    "stt.deepgram": "Deepgram (cloud)",
    "stt.assemblyai": "AssemblyAI (cloud)",
    "tr.none": "Off",
    "tr.local": "Local (offline, NLLB-200)",
    "tr.deepl": "DeepL (cloud)",
    "tr.openai": "OpenAI (cloud)",
    "tr.google": "Google Translate (cloud)",
    "tr.azure": "Azure Translator (cloud)",

    // Updates
    "update.check": "Check for updates",
    "update.checking": "Checking…",
    "update.upToDate": "You're up to date",
    "update.installing": "Installing update, restarting…",
    "update.failed": "Update check failed: {error}",

    // Stages
    "stage.model": "Model",
    "stage.audio": "Audio",
    "stage.transcript": "Transcript",
    "stage.subtitles": "Subtitles",

    // Body
    "drop.title": "Choose a video or audio file",
    "file.change": "Change",
    "generate.run": "Generate subtitles",
    "generate.running": "Generating…",
    "generate.failed": "Generation failed.",
    "result.lines_one": "{count} line",
    "result.lines_other": "{count} lines",
    "result.saveAs": "Save as…",

    // Translation
    "translate.autoDetect": "Auto (detect)",
    "translate.autoDetected": "Auto (use detected)",
    "translate.run": "Translate",
    "translate.running": "Translating…",
    "translate.failed": "Translation failed: {error}",
  },
  fr: {
    // Header
    "status.idle": "En attente",
    "status.running": "En cours",
    "status.done": "Terminé",
    "status.error": "Erreur",
    "header.settings": "Réglages",

    // Settings
    "settings.stt": "Moteur de transcription",
    "settings.translation": "Traduction",
    "settings.azureRegion": "Région Azure",
    "settings.language": "Langue",
    "settings.updates": "Mises à jour",
    "settings.version": "Version",
    "settings.apiKey": "Clé API",
    "settings.apiKeySaved": "enregistrée",
    "settings.save": "Enregistrer",

    // Engines
    "stt.local": "Local (hors ligne, whisper.cpp)",
    "stt.groq": "Groq (cloud)",
    "stt.openai": "OpenAI (cloud)",
    "stt.deepgram": "Deepgram (cloud)",
    "stt.assemblyai": "AssemblyAI (cloud)",
    "tr.none": "Désactivée",
    "tr.local": "Local (hors ligne, NLLB-200)",
    "tr.deepl": "DeepL (cloud)",
    "tr.openai": "OpenAI (cloud)",
    "tr.google": "Google Translate (cloud)",
    "tr.azure": "Azure Translator (cloud)",

    // Updates
    "update.check": "Rechercher les mises à jour",
    "update.checking": "Recherche…",
    "update.upToDate": "Vous êtes à jour",
    "update.installing": "Installation de la mise à jour, redémarrage…",
    "update.failed": "Échec de la recherche de mise à jour : {error}",

    // Stages
    "stage.model": "Modèle",
    "stage.audio": "Audio",
    "stage.transcript": "Transcription",
    "stage.subtitles": "Sous-titres",

    // Body
    "drop.title": "Choisissez un fichier vidéo ou audio",
    "file.change": "Changer",
    "generate.run": "Générer les sous-titres",
    "generate.running": "Génération…",
    "generate.failed": "La génération a échoué.",
    "result.lines_one": "{count} ligne",
    "result.lines_other": "{count} lignes",
    "result.saveAs": "Enregistrer sous…",

    // Translation
    "translate.autoDetect": "Auto (détection)",
    "translate.autoDetected": "Auto (langue détectée)",
    "translate.run": "Traduire",
    "translate.running": "Traduction…",
    "translate.failed": "La traduction a échoué : {error}",
  },
};

export function getT(lang: Lang) {
  return (key: string, vars?: Record<string, string | number>): string => {
    let s = dict[lang][key] ?? dict.en[key] ?? key;
    if (vars)
      for (const [k, v] of Object.entries(vars)) s = s.replace(`{${k}}`, String(v));
    return s;
  };
}

/** Les deux langues n'accordent pas le pluriel au même seuil : l'anglais met
 * « 0 lines » au pluriel, le français écrit « 0 ligne » au singulier. */
export function getPlural(lang: Lang) {
  const t = getT(lang);
  return (key: string, count: number): string => {
    const isPlural = lang === "fr" ? count > 1 : count !== 1;
    return t(`${key}_${isPlural ? "other" : "one"}`, { count });
  };
}
