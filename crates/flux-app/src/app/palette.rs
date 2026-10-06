//! Palette de commandes (Ctrl K) : toutes les actions de l'application, filtrées au clavier.

use super::{App, EXAMPLES, Panel, TOOLS, blank, example};
use crate::lang::{Lang, tr};
use crate::theme::{self as t, mono, sans};
use crate::ui::{self, PAD};
use crate::units::Units;
use eframe::egui::{self, Align2, Key, Modifiers, Rect, Sense, Stroke, Vec2, pos2, vec2};
use flux_core::DVec2;
use flux_core::scene::MechView;

/// État de la palette ouverte : texte recherché et ligne en surbrillance.
#[derive(Default)]
pub(super) struct Palette {
    query: String,
    index: usize,
    /// La palette a déjà été affichée au moins une image.
    shown: bool,
}

#[derive(Clone, Copy)]
enum Command {
    Tool(usize),
    Mode(usize),
    Example(usize),
    New,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    Frame,
    ToggleUi,
    Units(Units),
    Lang(Lang),
    Fold(Panel),
    Swap(Panel),
    Duplicate,
    Delete,
    ToggleVisible,
    ToggleLocked,
    ClearSeeds,
    ClearFilings,
    ClearCut,
    Freeze,
    Play,
    Step,
    Rewind,
    View(MechView),
    ExportFemm,
    Remagnetize,
    Ambient,
}

/// Texte en minuscules sans accents, pour une recherche tolérante.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

impl App {
    /// Toutes les commandes : libellé dans la langue courante, raccourci, action.
    fn commands(&mut self) -> Vec<(String, &'static str, Command)> {
        const DIGITS: [&str; 7] = ["1", "2", "3", "4", "5", "6", "7"];
        const TOOL_KEYS: [&str; 16] = ["V", "R", "E", "O", "A", "P", "B", "M", "C", "H", "L", "G", "S", "N", "T", "Y"];
        let mut list = Vec::new();
        for (i, tool) in TOOLS.iter().enumerate() {
            list.push((format!("{} · {}", tr("Outil"), tr(tool.1)), TOOL_KEYS[i], Command::Tool(i)));
        }
        for (i, (name, _)) in self.modes().into_iter().enumerate() {
            list.push((format!("{} · {}", tr("Affichage"), tr(name)), DIGITS[i], Command::Mode(i)));
        }
        for (i, name) in EXAMPLES.into_iter().enumerate() {
            list.push((format!("{} · {}", tr("Exemple"), tr(name)), "", Command::Example(i)));
        }
        let simple = [
            ("Nouvelle scène", "", Command::New),
            ("Ouvrir une scène…", "", Command::Open),
            ("Enregistrer", "Ctrl S", Command::Save),
            ("Enregistrer sous…", "", Command::SaveAs),
            ("Annuler", "Ctrl Z", Command::Undo),
            ("Rétablir", "Ctrl Maj Z", Command::Redo),
            ("Cadrer la scène", "F", Command::Frame),
            ("Masquer ou afficher l'interface", "Tab", Command::ToggleUi),
            ("Unités SI (T, N)", "", Command::Units(Units::Si)),
            ("Unités CGS (G, dyn)", "", Command::Units(Units::Cgs)),
            ("Langue : français", "", Command::Lang(Lang::Fr)),
            ("Langue : anglais", "", Command::Lang(Lang::En)),
            ("Replier ou déplier la bibliothèque", "", Command::Fold(Panel::Library)),
            ("Replier ou déplier l'inspecteur", "", Command::Fold(Panel::Inspector)),
            ("Replier ou déplier le graphe de coupe", "", Command::Fold(Panel::Graph)),
            ("Ancrer la bibliothèque de l'autre côté", "", Command::Swap(Panel::Library)),
            ("Ancrer l'inspecteur de l'autre côté", "", Command::Swap(Panel::Inspector)),
            ("Ancrer le graphe de coupe de l'autre côté", "", Command::Swap(Panel::Graph)),
            ("Dupliquer la sélection", "Ctrl D", Command::Duplicate),
            ("Supprimer la sélection", "Suppr", Command::Delete),
            ("Masquer ou afficher la sélection", "", Command::ToggleVisible),
            ("Verrouiller ou déverrouiller la sélection", "", Command::ToggleLocked),
            ("Effacer les graines", "", Command::ClearSeeds),
            ("Balayer la limaille", "", Command::ClearFilings),
            ("Effacer la ligne de coupe", "", Command::ClearCut),
            ("Figer l’état actuel comme référence", "", Command::Freeze),
            ("Lecture ou pause de la simulation", "Espace", Command::Play),
            ("Avancer la simulation d’un pas", ".", Command::Step),
            ("Revenir à l’état initial", "", Command::Rewind),
            ("Vue de dessus (table)", "", Command::View(MechView::Top)),
            ("Vue de côté (pesanteur dans le plan)", "", Command::View(MechView::Side)),
            ("Ré-aimanter tous les aimants", "", Command::Remagnetize),
            ("Ramener tous les objets à la température ambiante", "", Command::Ambient),
            ("Exporter la scène pour FEMM (.lua)…", "", Command::ExportFemm),
        ];
        list.extend(simple.map(|(label, key, command)| (tr(label).to_owned(), key, command)));
        list
    }

    fn run(&mut self, command: Command) {
        let selected = self.selected;
        match command {
            Command::Tool(i) => self.tool = TOOLS[i].0,
            Command::Mode(i) => {
                let flag = self.modes().into_iter().nth(i).unwrap().1;
                *flag = !*flag;
            }
            Command::Example(i) => self.load(example(i), None),
            Command::New => self.load(blank(), None),
            Command::Open => self.open(),
            Command::Save => self.save(false),
            Command::SaveAs => self.save(true),
            Command::Undo => self.do_undo(),
            Command::Redo => self.do_redo(),
            Command::Frame => self.frame_all(),
            Command::ToggleUi => self.show_ui = !self.show_ui,
            Command::Units(units) => self.prefs.units = units,
            Command::Lang(lang) => self.prefs.lang = lang,
            Command::Fold(panel) => {
                let dock = self.prefs.dock(panel);
                dock.open = !dock.open;
            }
            Command::Swap(panel) => {
                let dock = self.prefs.dock(panel);
                dock.side = dock.side.opposite();
            }
            Command::Duplicate => self.selected = selected.and_then(|id| self.scene.duplicate(id, DVec2::splat(0.005))).or(selected),
            Command::Delete => self.delete_selected(),
            Command::ToggleVisible | Command::ToggleLocked => {
                if let Some(o) = selected.and_then(|id| self.scene.get_mut(id)) {
                    if matches!(command, Command::ToggleVisible) { o.visible = !o.visible } else { o.locked = !o.locked }
                }
            }
            Command::ClearSeeds => self.scene.seeds.clear(),
            Command::ClearFilings => self.visuals.filings.clear(),
            Command::ClearCut => self.scene.cut_line = None,
            Command::Freeze => self.freeze_reference(),
            Command::Play => self.toggle_play(),
            Command::Step => self.step_simulation(),
            Command::Rewind => self.rewind(),
            Command::View(view) => self.scene.mechanics.view = view,
            Command::ExportFemm => self.export_femm(),
            Command::Remagnetize => self.scene.remagnetize(None),
            Command::Ambient => {
                let ambient = self.scene.ambient;
                self.scene.objects.iter_mut().for_each(|o| o.temperature = ambient);
            }
        }
    }

    /// Affiche la palette si elle est ouverte : champ de recherche, liste filtrée, navigation au clavier.
    pub(super) fn show_palette(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.palette.take() else { return };
        let terms: Vec<String> = fold(&state.query).split_whitespace().map(str::to_owned).collect();
        let found: Vec<(String, &str, Command)> =
            self.commands().into_iter().filter(|(label, ..)| terms.iter().all(|term| fold(label).contains(term))).collect();

        // Flèches, Entrée et Échap sont interceptées avant le champ de texte.
        let (up, down, enter, escape) = ctx.input_mut(|i| {
            let mut pressed = |key| i.consume_key(Modifiers::NONE, key);
            (pressed(Key::ArrowUp), pressed(Key::ArrowDown), pressed(Key::Enter), pressed(Key::Escape))
        });
        let last = found.len().saturating_sub(1);
        let moved = if down { state.index + 1 } else { state.index.saturating_sub(up as usize) };
        state.index = moved.min(last);
        let mut chosen = found.get(state.index).filter(|_| enter).map(|(.., command)| *command);
        let mut close = escape;

        let screen = ctx.content_rect();
        let backdrop = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("palette-backdrop")));
        backdrop.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(140));
        let width = 520.0_f32.min(screen.width() - 40.0);
        let area = egui::Area::new(egui::Id::new("palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(screen.center().x - width / 2.0, screen.top() + 96.0))
            .show(ctx, |ui| {
                egui::Frame::new().fill(t::PANEL).stroke(Stroke::new(1.0, t::LINE_CTRL)).show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let p = ui.painter().clone();
                    let search = ui::row(ui, 40.0, t::LINE);
                    self.icons.search.paint(
                        &p,
                        Rect::from_center_size(pos2(search.left() + PAD + 6.5, search.center().y), Vec2::splat(13.0)),
                        1.3,
                        t::ACCENT,
                    );
                    let field =
                        Rect::from_min_max(pos2(search.left() + PAD + 23.0, search.top()), pos2(search.right() - PAD, search.bottom()));
                    let edit = ui::place(ui, field, egui::Layout::left_to_right(egui::Align::Center)).add(
                        egui::TextEdit::singleline(&mut state.query)
                            .frame(egui::Frame::NONE)
                            .hint_text(tr("Rechercher une commande"))
                            .font(sans(13.0))
                            .desired_width(field.width())
                            .margin(egui::Margin::ZERO),
                    );
                    edit.request_focus();
                    if edit.changed() {
                        state.index = 0;
                    }
                    egui::ScrollArea::vertical().max_height(30.0 * 11.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        // Peintre de la zone défilante : les lignes hors du cadre sont rognées.
                        let p = ui.painter().clone();
                        for (k, (label, key, command)) in found.iter().enumerate() {
                            let r = ui::row(ui, 30.0, t::LINE_SOFT);
                            let resp = ui.interact(r, ui.id().with(("command", k)), Sense::click());
                            let on = k == state.index;
                            if on || resp.hovered() {
                                p.rect_filled(r.shrink2(vec2(0.0, 0.5)), 0.0, if on { t::tint() } else { t::tint().gamma_multiply(0.5) });
                            }
                            if on {
                                ui::grad_v(&p, Rect::from_min_size(r.min, vec2(2.0, r.height())));
                                if up || down {
                                    ui.scroll_to_rect(r, None);
                                }
                            }
                            p.text(
                                pos2(r.left() + PAD, r.center().y),
                                Align2::LEFT_CENTER,
                                label,
                                sans(12.5),
                                if on { t::TEXT_HI } else { t::TEXT_MID },
                            );
                            // Seuls les noms de touches propres au français changent avec la langue.
                            let key = match *key {
                                "Ctrl Maj Z" | "Suppr" | "Espace" => tr(key),
                                key => key,
                            };
                            p.text(
                                pos2(r.right() - PAD, r.center().y),
                                Align2::RIGHT_CENTER,
                                key,
                                mono(10.5),
                                if on { t::ACCENT_TEXT } else { t::FAINT },
                            );
                            if resp.clicked() {
                                chosen = Some(*command);
                            }
                        }
                        if found.is_empty() {
                            let r = ui::row(ui, 30.0, t::LINE_SOFT);
                            p.text(pos2(r.left() + PAD, r.center().y), Align2::LEFT_CENTER, tr("Aucune commande"), sans(12.5), t::DIM);
                        }
                    });
                });
            });
        // Le clic qui a ouvert la palette ne doit pas la refermer aussitôt.
        close |= state.shown && area.response.clicked_elsewhere();
        state.shown = true;

        if let Some(command) = chosen {
            self.run(command);
        } else if !close {
            self.palette = Some(state);
        }
    }
}
