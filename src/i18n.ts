import { suiteMessages } from "./suite/messages";

export type Lang = "en" | "fr";

const dict: Record<Lang, Record<string, string>> = {
  en: {
    ...suiteMessages.en,
    "about.tagline": "Turn any video or audio file into subtitles, offline by default.",
    "menu.open": "Choose a file…",
    "menu.openRecent": "Open recent",
    // Header

    // Settings
    "settings.title": "Settings",
    "settings.stt": "Transcription engine",
    "settings.translation": "Translation",
    "settings.azureRegion": "Azure region",
    "settings.apiKey": "API key",
    "settings.apiKeySaved": "saved",
    "settings.save": "Save",

    // Engines
    "stt.local": "Local (offline, whisper.cpp)",
    "stt.groq": "Groq (cloud)",
    "stt.openai": "OpenAI (cloud)",
    "stt.deepgram": "Deepgram (cloud)",
    "stt.assemblyai": "AssemblyAI (cloud)",
    "tr.local": "Local (offline, NLLB-200)",
    "tr.deepl": "DeepL (cloud)",
    "tr.openai": "OpenAI (cloud)",
    "tr.google": "Google Translate (cloud)",
    "tr.azure": "Azure Translator (cloud)",

    // Updates

    // Stages
    "stage.model": "Model",
    "stage.audio": "Audio",
    "stage.transcript": "Transcript",
    "stage.subtitles": "Subtitles",

    // Body
    "drop.subtitle": "Drop a video or audio file here",
    "drop.or": "or",
    "drop.browse": "Browse files",
    "recent.title": "Recent",
    "recent.clear": "Clear",
    "recent.remove": "Remove from the list",
    "file.change": "Change",
    "generate.run": "Generate subtitles",
    "generate.running": "Generating…",
    "generate.failed": "Generation failed.",
    "result.lines_one": "{count} line",
    "result.lines_other": "{count} lines",

    // Translation
    "translate.autoDetect": "Auto (detect)",
    "translate.autoDetected": "Auto (use detected)",
    "translate.run": "Translate",
    "translate.running": "Translating…",
    "translate.failed": "Translation failed: {error}",

    // Dialogs
    "dialog.mediaFilter": "Video / Audio",
    "dialog.srtFilter": "SRT subtitles",
    "dialog.vttFilter": "WebVTT subtitles",

    // Settings (transcription)
    "settings.model": "Whisper model",
    "settings.sttLanguage": "Spoken language",
    "settings.cloudNotice": "A cloud engine uploads your audio to its provider.",
    "settings.remove": "Remove",
    "settings.keyError": "Could not update the key: {error}",
    "model.base": "Base (60 MB, fastest)",
    "model.small": "Small (190 MB, recommended)",
    "model.medium": "Medium (540 MB, most accurate)",
    "stt.langAuto": "Auto-detect",

    // Body
    "drop.dragging": "Drop the file to select it",
    "file.unsupported": "This file type is not supported: {name}",
    "generate.regenerate": "Regenerate",
    "generate.cancel": "Cancel",
    "generate.keyMissing": "Add your API key in Settings before generating.",
    "progress.label": "Progress",
    "translate.same": "The source and target languages are the same.",
    "translate.keyMissing": "Add your API key in Settings before translating.",
    "translate.source": "Source language",
    "translate.target": "Target language",

    // Errors coming from the engine
    "error.ffmpeg_missing":
      "ffmpeg was not found. Install it (macOS: brew install ffmpeg · Debian/Ubuntu: sudo apt install ffmpeg · Fedora: sudo dnf install ffmpeg-free · Arch: sudo pacman -S ffmpeg) and restart the app.",
    "error.no_audio": "This file has no audio track.",
    "error.file_not_found": "The file could not be found.",
    "error.api_key_missing": "No API key is saved for this engine — add one in Settings.",
    "error.api_key_invalid":
      "The provider refused the request: the API key is wrong, or the service is not enabled for it. Check it in Settings. {detail}",
    "error.quota_exceeded": "The provider's quota or rate limit was reached. {detail}",
    "error.file_too_large": "The provider refused the file because it is too large. {detail}",
    "error.azure_region_missing": "The Azure region is not set — add it in Settings.",
    "error.no_cues": "There are no subtitles to translate.",
    "error.source_lang_required":
      "The local translator needs to know the source language — pick it instead of “Auto”.",
    "error.unsupported_language": "This language is not supported.",
    "error.download_corrupt": "The downloaded model failed its integrity check. Try again.",
    "error.unknown_key": "Unknown API key.",

    // Audio tracks, ASS export, editing
    "audio.track": "Audio track",
    "audio.default": "Default track",
    "dialog.assFilter": "ASS subtitles",
    "edit.start": "Edit",
    "edit.apply": "Apply changes",
    "edit.discard": "Discard",
    "edit.cue": "Text of cue {index}",
    "edit.failed": "Could not apply the changes: {error}",
    "error.path_not_allowed": "This file was not chosen in the app. Pick it again.",
    "error.network_unreachable":
      "The provider could not be reached. Check your internet connection. {detail}",
    "error.provider_unavailable":
      "The provider is having trouble (server error). Try again in a moment. {detail}",

    // Saving, progress, editing
    "export.button": "Export",
    "export.title": "Export subtitles",
    "export.subtitles": "Subtitles",
    "export.original": "Original",
    "export.translation": "Translation",
    "export.format": "Format",
    "export.confirm": "Export",
    "export.cancel": "Cancel",
    "result.saved": "Saved to {path}",
    "result.saveFailed": "Could not save the file: {error}",
    "settings.saveFailed": "Could not save the settings: {error}",
    "generate.cancelling": "Cancelling…",
    "progress.elapsed": "Elapsed {time}",
    "progress.remaining": "about {time} left",
    "edit.emptyWarning": "{count} empty cue(s) will be removed from the subtitles.",
    "edit.confirmRemove": "Remove them and apply",
  },
  fr: {
    ...suiteMessages.fr,
    "about.tagline":
      "Transformez n'importe quel fichier vidéo ou audio en sous-titres, hors ligne par défaut.",
    "menu.open": "Choisir un fichier…",
    "menu.openRecent": "Ouvrir récent",
    // Header

    // Settings
    "settings.title": "Réglages",
    "settings.stt": "Moteur de transcription",
    "settings.translation": "Traduction",
    "settings.azureRegion": "Région Azure",
    "settings.apiKey": "Clé API",
    "settings.apiKeySaved": "enregistrée",
    "settings.save": "Enregistrer",

    // Engines
    "stt.local": "Local (hors ligne, whisper.cpp)",
    "stt.groq": "Groq (cloud)",
    "stt.openai": "OpenAI (cloud)",
    "stt.deepgram": "Deepgram (cloud)",
    "stt.assemblyai": "AssemblyAI (cloud)",
    "tr.local": "Local (hors ligne, NLLB-200)",
    "tr.deepl": "DeepL (cloud)",
    "tr.openai": "OpenAI (cloud)",
    "tr.google": "Google Translate (cloud)",
    "tr.azure": "Azure Translator (cloud)",

    // Updates

    // Stages
    "stage.model": "Modèle",
    "stage.audio": "Audio",
    "stage.transcript": "Transcription",
    "stage.subtitles": "Sous-titres",

    // Body
    "drop.subtitle": "Déposez un fichier vidéo ou audio ici",
    "drop.or": "ou",
    "drop.browse": "Parcourir les fichiers",
    "recent.title": "Récents",
    "recent.clear": "Effacer",
    "recent.remove": "Retirer de la liste",
    "file.change": "Changer",
    "generate.run": "Générer les sous-titres",
    "generate.running": "Génération…",
    "generate.failed": "La génération a échoué.",
    "result.lines_one": "{count} ligne",
    "result.lines_other": "{count} lignes",

    // Translation
    "translate.autoDetect": "Auto (détection)",
    "translate.autoDetected": "Auto (langue détectée)",
    "translate.run": "Traduire",
    "translate.running": "Traduction…",
    "translate.failed": "La traduction a échoué : {error}",

    // Dialogs
    "dialog.mediaFilter": "Vidéo / Audio",
    "dialog.srtFilter": "Sous-titres SRT",
    "dialog.vttFilter": "Sous-titres WebVTT",

    // Settings (transcription)
    "settings.model": "Modèle Whisper",
    "settings.sttLanguage": "Langue parlée",
    "settings.cloudNotice": "Un moteur cloud envoie votre audio à son fournisseur.",
    "settings.remove": "Supprimer",
    "settings.keyError": "Impossible de mettre à jour la clé : {error}",
    "model.base": "Base (60 Mo, le plus rapide)",
    "model.small": "Small (190 Mo, recommandé)",
    "model.medium": "Medium (540 Mo, le plus précis)",
    "stt.langAuto": "Détection automatique",

    // Body
    "drop.dragging": "Déposez le fichier pour le sélectionner",
    "file.unsupported": "Ce type de fichier n'est pas pris en charge : {name}",
    "generate.regenerate": "Régénérer",
    "generate.cancel": "Annuler",
    "generate.keyMissing": "Ajoutez votre clé API dans les réglages avant de générer.",
    "progress.label": "Progression",
    "translate.same": "La langue source et la langue cible sont identiques.",
    "translate.keyMissing": "Ajoutez votre clé API dans les réglages avant de traduire.",
    "translate.source": "Langue source",
    "translate.target": "Langue cible",

    // Errors coming from the engine
    "error.ffmpeg_missing":
      "ffmpeg est introuvable. Installez-le (macOS : brew install ffmpeg · Debian/Ubuntu : sudo apt install ffmpeg · Fedora : sudo dnf install ffmpeg-free · Arch : sudo pacman -S ffmpeg) puis relancez l'application.",
    "error.no_audio": "Ce fichier ne contient pas de piste audio.",
    "error.file_not_found": "Le fichier est introuvable.",
    "error.api_key_missing":
      "Aucune clé API enregistrée pour ce moteur — ajoutez-en une dans les réglages.",
    "error.api_key_invalid":
      "Le fournisseur a refusé la requête : la clé API est incorrecte, ou le service n'est pas activé pour elle. Vérifiez-la dans les réglages. {detail}",
    "error.quota_exceeded": "Le quota ou la limite de débit du fournisseur est atteint. {detail}",
    "error.file_too_large":
      "Le fournisseur a refusé le fichier car il est trop volumineux. {detail}",
    "error.azure_region_missing":
      "La région Azure n'est pas renseignée — ajoutez-la dans les réglages.",
    "error.no_cues": "Il n'y a aucun sous-titre à traduire.",
    "error.source_lang_required":
      "Le traducteur local doit connaître la langue source — choisissez-la plutôt que « Auto ».",
    "error.unsupported_language": "Cette langue n'est pas prise en charge.",
    "error.download_corrupt":
      "Le modèle téléchargé a échoué à la vérification d'intégrité. Réessayez.",
    "error.unknown_key": "Clé API inconnue.",

    // Audio tracks, ASS export, editing
    "audio.track": "Piste audio",
    "audio.default": "Piste par défaut",
    "dialog.assFilter": "Sous-titres ASS",
    "edit.start": "Modifier",
    "edit.apply": "Appliquer les modifications",
    "edit.discard": "Annuler",
    "edit.cue": "Texte de la cue {index}",
    "edit.failed": "Impossible d'appliquer les modifications : {error}",
    "error.path_not_allowed":
      "Ce fichier n'a pas été choisi dans l'application. Sélectionnez-le à nouveau.",
    "error.network_unreachable":
      "Le fournisseur est injoignable. Vérifiez votre connexion internet. {detail}",
    "error.provider_unavailable":
      "Le fournisseur rencontre un problème (erreur serveur). Réessayez dans un instant. {detail}",

    // Saving, progress, editing
    "export.button": "Exporter",
    "export.title": "Exporter les sous-titres",
    "export.subtitles": "Sous-titres",
    "export.original": "Original",
    "export.translation": "Traduction",
    "export.format": "Format",
    "export.confirm": "Exporter",
    "export.cancel": "Annuler",
    "result.saved": "Enregistré dans {path}",
    "result.saveFailed": "Impossible d'enregistrer le fichier : {error}",
    "settings.saveFailed": "Impossible d'enregistrer les réglages : {error}",
    "generate.cancelling": "Annulation…",
    "progress.elapsed": "Écoulé : {time}",
    "progress.remaining": "environ {time} restantes",
    "edit.emptyWarning": "{count} cue(s) vide(s) seront retirées des sous-titres.",
    "edit.confirmRemove": "Les retirer et appliquer",
  },
};

/** Every key of the English dictionary, to check the other languages stay complete. */
export function translationKeys(lang: Lang): string[] {
  return Object.keys(dict[lang]);
}

export function getT(lang: Lang) {
  return (key: string, vars?: Record<string, string | number>): string => {
    let s = dict[lang][key] ?? dict.en[key] ?? key;
    if (vars) for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
    // A placeholder left without a value (e.g. an optional {detail}) must not show up.
    return s.replace(/\s*\{\w+\}/g, "").trim();
  };
}

/** The two languages do not apply the plural at the same threshold: English makes
 * "0 lines" plural, French writes "0 ligne" in the singular. */
export function getPlural(lang: Lang) {
  const t = getT(lang);
  return (key: string, count: number): string => {
    const isPlural = lang === "fr" ? count > 1 : count !== 1;
    return t(`${key}_${isPlural ? "other" : "one"}`, { count });
  };
}
