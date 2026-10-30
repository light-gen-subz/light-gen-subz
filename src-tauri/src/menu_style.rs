//! Paints the GTK menu bar and menus like LightCutVidz's (measured on its screenshots: the
//! bar is the title bar's grey and 29 px tall; entries are 33 px tall, set in 15.7 px with
//! 19 px at each side; a popup has 10 px above and below), so that the apps of the suite
//! look alike. They stay dark whatever the theme of the page: LightCutVidz's menus are, and
//! GTK draws them with the system's theme. Same sheet as LightCutSoundz; only the accent of
//! the checked entries differs.

use gtk::prelude::{CssProviderExt, GtkSettingsExt};

/// This app's primary colour, for the dot of the checked entry (language, theme).
const ACCENT: &str = "#ffb454";

/// Chromium (LightCutVidz) draws its text unhinted; GTK follows the system's "slight"
/// hinting, which makes the glyphs a pixel shorter. Before the first window: the web view
/// reads the font settings when it is created.
pub fn unhinted_text() {
    if gtk::init().is_err() {
        return;
    }
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_xft_hinting(0);
        settings.set_gtk_xft_hintstyle(Some("hintnone"));
    }
}

fn css() -> String {
    // GTK reserves 25 px before an entry's text and `muda` pulls a submenu's box back by a
    // fixed -22 px. The negative margins below bring every label to 20 px from the popup's
    // edge, as in LightCutVidz; USER priority lets this sheet win over `muda`'s own rule.
    format!(
        "\
        menubar {{ background-color: #2c2c2c; min-height: 29px; }} \
        menubar > menuitem {{ min-height: 29px; padding-top: 0; padding-bottom: 0; }} \
        menubar > menuitem label {{ color: #f7f7f7; }} \
        .csd.popup decoration {{ box-shadow: 0 1px 2px transparent, 0 0 0 1px transparent; }} \
        menu {{ padding: 10px 0; border: none; border-radius: 8px; box-shadow: none; background-color: #1d1d1d; }} \
        menu menuitem {{ min-height: 33px; padding: 0 19px 0 0; margin-left: -5px; font-size: 15.7px; }} \
        menu menuitem:hover {{ background-color: #3a3a3a; }} \
        menu menuitem > box {{ margin: 0 0 0 -25px; }} \
        menu menuitem > box > image {{ min-width: 0; margin: 0 0 0 3px; }} \
        menu menuitem label {{ color: #f7f7f7; }} \
        menu menuitem check {{ min-width: 16px; min-height: 16px; margin-left: 16px; margin-right: 12px; border: 2px solid #f7f7f7; border-radius: 50%; background-color: transparent; background-image: none; box-shadow: none; -gtk-icon-source: none; }} \
        menu menuitem check:checked {{ border-color: {ACCENT}; background-image: radial-gradient(circle, {ACCENT} 38%, transparent 42%); }} \
        menu menuitem:disabled label {{ color: #929292; }} \
        menu separator {{ margin: 8px 0; }}"
    )
}

pub fn style_menu_bar() {
    let provider = gtk::CssProvider::new();
    if provider.load_from_data(css().as_bytes()).is_err() {
        return;
    }
    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_carries_the_app_accent_and_is_not_empty() {
        let css = css();
        assert!(css.contains(ACCENT));
        assert!(css.contains("menubar"));
        assert!(!css.contains("{ACCENT}"));
    }
}
