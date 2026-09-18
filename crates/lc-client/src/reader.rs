//! The book, drawn.
//!
//! Every other surface in this client is a readout over a rendered sky and is dressed to look
//! like one. This one is dressed to look like a thing you hold: a dark case with a page in it,
//! black on a gentle white, because a player should know what it is before reading a word of it.
//!
//! Two departures from [18-ui-style.md](../../lightcone/docs/18-ui-style.md), both deliberate
//! and both recorded there: the surface is opaque, and it is light. Prose over a drifting
//! starfield is unreadable in a way a readout over one is not.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Color32, CornerRadius, FontFamily, FontId, Margin, Rect, Stroke, Vec2};
use lc_books::paginate::{self, Cursor, Frame as PageFrame, Measure, Measured, Row};
use lc_books::{Block, Document};

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::library::{Book, Face, FontFace, READING_FACE, Shelf};
use crate::panels::ask;
use crate::ui::Panel;

/// The page. Not white: paper is not white, and a white panel on a dark screen is a lamp.
pub(crate) const PAPER: Color32 = Color32::from_rgb(244, 241, 233);
pub(crate) const INK: Color32 = Color32::from_rgb(26, 25, 23);
/// The running head and the folio, which are furniture rather than text.
pub(crate) const FAINT: Color32 = Color32::from_rgb(146, 141, 130);
pub(crate) const RULE: Color32 = Color32::from_rgb(214, 209, 197);
/// The case the page sits in.
pub(crate) const CASE: Color32 = Color32::from_rgb(38, 40, 44);
pub(crate) const CASE_EDGE: Color32 = Color32::from_rgb(58, 61, 67);
pub(crate) const KEY: Color32 = Color32::from_rgb(52, 55, 60);
pub(crate) const KEY_HOT: Color32 = Color32::from_rgb(70, 74, 80);
pub(crate) const LABEL: Color32 = Color32::from_rgb(198, 196, 190);

/// What the reading face is called once it has been installed.
pub const READING_FAMILY: &str = "reading";

const BODY_SIZE: f32 = 17.0;
/// Air above and below a plate, so it does not touch the text it interrupts.
const PLATE_GAP: f32 = 10.0;
/// What a plate of unknown shape reserves, and the box drawn while it is being read.
const PLATE_UNKNOWN: Vec2 = Vec2::new(4.0, 3.0);
/// The widest a plate is uploaded at. A transcription's plates are a few hundred pixels across;
/// a cover can be two thousand, and a texture of one is eight megabytes for a picture nobody
/// will look at closely.
const PLATE_TEXELS: u32 = 1400;
/// How many decoded plates are kept. One chapter of an illustrated edition holds a handful;
/// a book holds a hundred and seventy-eight, which is why this is a cap and not a map.
const PLATE_CACHE: usize = 12;
/// The bezel below the page, where the buttons are.
pub(crate) const KEYS_HEIGHT: f32 = 30.0;
pub(crate) const SCREEN_MARGIN: Margin = Margin { left: 30, right: 30, top: 26, bottom: 22 };

pub(crate) struct Setting {
    pub body: FontId,
    pub heading: FontId,
    pub small: FontId,
}

impl Setting {
    fn new(serif: bool) -> Self {
        let family =
            if serif { FontFamily::Name(READING_FAMILY.into()) } else { FontFamily::Proportional };
        Self {
            body: FontId::new(BODY_SIZE, family.clone()),
            heading: FontId::new(BODY_SIZE * 1.25, family),
            small: FontId::new(11.0, FontFamily::Proportional),
        }
    }
}

/// The measurer the pagination runs over: egui's own layout, at the size it will be drawn.
///
/// The same function lays out the galley that is painted, so what was measured and what is on
/// the page cannot disagree.
struct Setter<'a> {
    ctx: &'a egui::Context,
    setting: &'a Setting,
    /// The shape of each plate, from the chapter's own images.
    plates: &'a std::collections::HashMap<String, (u32, u32)>,
    /// The tallest anything on this page may be, which is what stops a cover from being
    /// measured at twice the height of the frame it has to fit in.
    ceiling: f32,
}

impl Measure for Setter<'_> {
    fn measure(&self, block: &Block, width: f32) -> Measured {
        let lead = match block {
            Block::Heading { .. } => BODY_SIZE * 1.4,
            Block::Item { .. } => BODY_SIZE * 0.2,
            Block::Rule => BODY_SIZE,
            _ => BODY_SIZE * 0.55,
        };
        if let Block::Image { path, .. } = block {
            let drawn = plate_size(self.plates.get(path), width, self.ceiling - PLATE_GAP * 2.0);
            return Measured { lead, rows: vec![Row { height: drawn.y + PLATE_GAP * 2.0, offset: 0 }] };
        }
        let galley = self.ctx.fonts_mut(|f| f.layout_job(job(block, width, self.setting)));
        let mut rows = Vec::with_capacity(galley.rows.len());
        let mut at = 0;
        for (i, placed) in galley.rows.iter().enumerate() {
            // From the positions rather than the sizes, because the painter draws from the
            // positions: a row's `pos.y` is rounded to the pixel grid and its `size.y` is not,
            // and measuring one while painting the other shows a sliver of the next line.
            let bottom = match galley.rows.get(i + 1) {
                Some(next) => next.pos.y,
                None => galley.rect.height(),
            };
            rows.push(Row { height: bottom - placed.pos.y, offset: at });
            at += placed.row.char_count_including_newline();
        }
        Measured { lead, rows }
    }
}

/// How big a plate is drawn: its own size, down to whatever fits.
///
/// Never larger than the image itself. A four-hundred-pixel woodcut blown up to fill a column
/// is a blurred woodcut, and the transcriptions on this shelf are full of them.
fn plate_size(natural: Option<&(u32, u32)>, column: f32, ceiling: f32) -> Vec2 {
    let (w, h) = match natural {
        Some((w, h)) => (*w.max(&1) as f32, *h.max(&1) as f32),
        None => (PLATE_UNKNOWN.x, PLATE_UNKNOWN.y),
    };
    let mut width = column.min(w);
    let mut height = width * h / w;
    if height > ceiling {
        height = ceiling.max(1.0);
        width = height * w / h;
    }
    Vec2::new(width.max(1.0), height.max(1.0))
}

/// How far a block is inset from the column, in points.
fn inset(block: &Block) -> f32 {
    match block {
        Block::Quote(_) => 24.0,
        Block::Item { depth, .. } => 18.0 + 14.0 * *depth as f32,
        _ => 0.0,
    }
}

fn job(block: &Block, width: f32, setting: &Setting) -> LayoutJob {
    let font = match block {
        Block::Heading { .. } => setting.heading.clone(),
        _ => setting.body.clone(),
    };
    let mut job = LayoutJob {
        wrap: TextWrapping { max_width: (width - inset(block)).max(32.0), ..Default::default() },
        break_on_newline: true,
        halign: Align::LEFT,
        ..Default::default()
    };
    if let Block::Item { ordered, .. } = block {
        let marker = if *ordered { "— " } else { "• " };
        job.append(marker, 0.0, TextFormat { font_id: font.clone(), color: FAINT, ..Default::default() });
    }
    if let Block::Rule = block {
        job.append("* * *", 0.0, TextFormat { font_id: font, color: FAINT, ..Default::default() });
        return job;
    }
    let Some(text) = block.text() else { return job };
    for run in &text.runs {
        job.append(
            &run.text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: if matches!(block, Block::Quote(_)) { FAINT } else { INK },
                italics: run.style.italic || matches!(block, Block::Quote(_)),
                ..Default::default()
            },
        );
    }
    job
}

/// Install the reading face, or settle for the interface font.
///
/// Asked for only when a book is first opened, so a player who never reads never downloads it.
fn settle_face(
    ctx: &egui::Context,
    shelf: &mut Shelf,
    assets: &AssetServer,
    faces: &Assets<FontFace>,
) {
    match &shelf.face {
        Face::Settled => {}
        Face::Unasked => {
            shelf.face = Face::Waiting(assets.load(READING_FACE));
        }
        Face::Waiting(handle) => {
            use bevy::asset::LoadState;
            match assets.get_load_state(handle) {
                Some(LoadState::Failed(_)) => {
                    info!("no reading face at {READING_FACE}; setting the page in the interface font");
                    shelf.face = Face::Settled;
                }
                Some(LoadState::Loaded) => {
                    if let Some(face) = faces.get(handle) {
                        let mut fonts = egui::FontDefinitions::default();
                        fonts.font_data.insert(
                            READING_FAMILY.to_owned(),
                            std::sync::Arc::new(egui::FontData::from_owned(face.0.clone())),
                        );
                        fonts
                            .families
                            .insert(FontFamily::Name(READING_FAMILY.into()), vec![READING_FAMILY.to_owned()]);
                        ctx.set_fonts(fonts);
                        info!("the page is set in {READING_FACE}");
                    }
                    shelf.face = Face::Settled;
                }
                _ => {}
            }
        }
    }
}

/// The faces to set a surface in, serif if this build has one.
pub(crate) fn setting_for(ctx: &egui::Context) -> Setting {
    Setting::new(has_serif(ctx))
}

fn has_serif(ctx: &egui::Context) -> bool {
    ctx.fonts(|f| f.families().iter().any(|family| *family == FontFamily::Name(READING_FAMILY.into())))
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut contexts: EguiContexts,
    mut state: ResMut<Ui>,
    mut shelf: ResMut<Shelf>,
    mut books: ResMut<Assets<Book>>,
    faces: Res<Assets<FontFace>>,
    assets: Res<AssetServer>,
    mut out: MessageWriter<Requested>,
    mut counted: Local<Counted>,
    mut plates: Local<Plates>,
) {
    if !state.is_open(Panel::Reader) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    settle_face(ctx, &mut shelf, &assets, &faces);
    let setting = setting_for(ctx);

    let view = ctx.content_rect();
    let width = 520.0_f32.min(view.width() - 48.0);
    let height = 680.0_f32.min(view.height() - 48.0);
    let mut open = true;
    egui::Window::new("reader")
        .title_bar(false)
        .resizable(true)
        .default_size([width, height])
        .default_pos(view.center() - Vec2::new(width, height) / 2.0)
        .max_height(view.height() - 24.0)
        .min_width(320.0)
        .min_height(280.0)
        .frame(case())
        .show(ctx, |ui| {
            // Measured once, before anything is drawn into it. Sizing content from what is left
            // *inside* a window that grows to fit its content is a loop, and the loop's fixed
            // point is a window taller than the screen.
            let inside = ui.available_height();
            head(ui, &shelf, &mut open, &mut out);
            let taken = ui.min_rect().height();
            let paper = (inside - taken - KEYS_HEIGHT - SCREEN_MARGIN.top as f32
                - SCREEN_MARGIN.bottom as f32)
                .max(80.0);
            let screen = egui::Frame {
                inner_margin: SCREEN_MARGIN,
                fill: PAPER,
                stroke: Stroke::new(1.0_f32, RULE),
                corner_radius: CornerRadius::same(3),
                ..Default::default()
            };
            screen.show(ui, |ui| {
                ui.set_height(paper);
                ui.set_width(ui.available_width());
                if state.reading.contents {
                    contents(ui, &shelf, &mut books, &setting, &mut out);
                } else {
                    page(ui, &mut state, &shelf, &mut books, &setting, &mut counted, &mut plates);
                }
            });
            ui.add_space(6.0);
            keys(ui, &state, &shelf, &mut out);
        });
    if !open {
        ask(&mut out, Action::CloseBook);
    }
}

pub(crate) fn case() -> egui::Frame {
    egui::Frame {
        inner_margin: Margin::same(12),
        fill: CASE,
        stroke: Stroke::new(1.0_f32, CASE_EDGE),
        corner_radius: CornerRadius::same(14),
        shadow: egui::epaint::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(120),
        },
        ..Default::default()
    }
}

/// The top of the case: what is being read, and the way out.
pub(crate) fn settle_face_for_shelf(
    ctx: &egui::Context,
    shelf: &mut Shelf,
    assets: &AssetServer,
    faces: &Assets<FontFace>,
) {
    settle_face(ctx, shelf, assets, faces);
}

fn head(ui: &mut egui::Ui, shelf: &Shelf, open: &mut bool, out: &mut MessageWriter<Requested>) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.add(engraved(shelf.title.to_uppercase(), 10.0, LABEL));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if key(ui, "×", "close the book").clicked() {
                *open = false;
            }
            if key(ui, "contents", "the chapter list").clicked() {
                ask(out, Action::ToggleContents);
            }
            if key(ui, "shelf", "the other books").clicked() {
                ask(out, Action::TogglePanel(Panel::Bookshelf));
            }
        });
    });
    ui.add_space(8.0);
}

/// The buttons along the foot of the case, which are the only ones a reader needs.
fn keys(ui: &mut egui::Ui, state: &Ui, shelf: &Shelf, out: &mut MessageWriter<Requested>) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        if key(ui, "‹  previous", "back a page").clicked() {
            ask(out, Action::TurnPage(-1));
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if key(ui, "next  ›", "on a page").clicked() {
                ask(out, Action::TurnPage(1));
            }
            // Of the whole book, not of this chapter: a location is what survives being told
            // to someone else, and "location 41 of 878" means the same thing on every machine.
            let words = match shelf.location(state.reading.spine, state.reading.offset) {
                Some((at, of)) => format!("location {at} of {of}"),
                None => "measuring".to_owned(),
            };
            ui.add(engraved(words, 10.0, FAINT));
        });
    });
}

/// Writing on the case rather than text on the page.
///
/// egui makes a label selectable by default, and a selectable label eats the drag that would
/// otherwise move the window — so the one surface a player grabs to move the thing is the one
/// that refuses to be grabbed. Nothing printed on the bezel is text anyone wants to copy.
pub(crate) fn engraved(words: impl Into<String>, size: f32, colour: Color32) -> egui::Label {
    egui::Label::new(egui::RichText::new(words.into()).size(size).color(colour)).selectable(false)
}

pub(crate) fn key(ui: &mut egui::Ui, label: &str, hint: &str) -> egui::Response {
    let widgets = &mut ui.style_mut().visuals.widgets;
    widgets.inactive.weak_bg_fill = KEY;
    widgets.hovered.weak_bg_fill = KEY_HOT;
    widgets.active.weak_bg_fill = KEY_HOT;
    widgets.inactive.corner_radius = CornerRadius::same(4);
    widgets.hovered.corner_radius = CornerRadius::same(4);
    widgets.active.corner_radius = CornerRadius::same(4);
    ui.add(egui::Button::new(egui::RichText::new(label).size(12.0).color(LABEL)))
        .on_hover_text(hint)
}

/// The page itself: lay it out where the reader is, turn it if they asked, then paint it.
/// How many pages this chapter has at this size, counted once and kept until one of them
/// changes. Counting is a whole-chapter pagination, which is the one place this client does
/// that — and it is why it is cached rather than done per frame.
#[derive(Default)]
pub struct Counted {
    key: Option<(usize, u32, u32)>,
    pages: usize,
}

#[allow(clippy::too_many_arguments)]
fn page(
    ui: &mut egui::Ui,
    state: &mut Ui,
    shelf: &Shelf,
    books: &mut Assets<Book>,
    setting: &Setting,
    counted: &mut Counted,
    plates: &mut Plates,
) {
    let Some((spine, doc)) = &shelf.open else {
        waiting(ui, shelf, setting);
        return;
    };
    // A chapter is fetched and parsed a frame after it is asked for, and a page turn spent
    // against the chapter the reader has just left runs off the end of the wrong one.
    if *spine != state.reading.spine.min(shelf.spine_count.saturating_sub(1)) {
        waiting(ui, shelf, setting);
        return;
    }
    let width = ui.available_width();
    let height = (ui.available_height() - 22.0).max(60.0);
    let frame = PageFrame { width, height };
    // Cloned because it is an `Arc` inside and the alternative is holding a borrow of the `Ui`
    // across everything below, which is the one thing a `Ui` will not allow.
    let ctx = ui.ctx().clone();
    let setter = Setter { ctx: &ctx, setting, plates: &shelf.plates, ceiling: height };

    // A page turn is spent here rather than in `action::apply`, because turning one means
    // laying it out and this is the only place with the fonts to do that.
    let mut cursor = start_cursor(doc, &setter, frame, state.reading.offset);
    let mut current = paginate::page_at(doc, &setter, frame, cursor);
    let turns = std::mem::take(&mut state.reading.turn);
    for _ in 0..turns.abs() {
        if turns > 0 {
            if current.next.block >= doc.blocks.len() {
                // Off the end of a chapter is the head of the next one, and off the end of the
                // last is where the book stops.
                if *spine + 1 < shelf.spine_count {
                    state.reading.spine = spine + 1;
                    state.reading.offset = 0;
                }
                break;
            }
            current = paginate::page_at(doc, &setter, frame, current.next);
        } else {
            match paginate::page_before(doc, &setter, frame, current.cursor()) {
                Some(before) => current = before,
                None => {
                    if *spine > 0 {
                        state.reading.spine = spine - 1;
                        state.reading.offset = usize::MAX;
                    }
                    break;
                }
            }
        }
        cursor = current.cursor();
    }
    let _ = cursor;
    // The reader writes down where it ended up. Nothing else can: the offset is a fact about a
    // layout, and this is the only place that has one.
    state.reading.offset = current.start;

    let key = (*spine, frame.width.to_bits(), frame.height.to_bits());
    if counted.key != Some(key) {
        counted.key = Some(key);
        counted.pages = paginate::pages(doc, &setter, frame).count();
    }
    let folio_number = paginate::pages(doc, &setter, frame)
        .position(|p| p.start >= current.start)
        .map(|i| i + 1)
        .unwrap_or(1);

    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let mut y = rect.top();
    for slice in &current.slices {
        let block = &doc.blocks[slice.block].block;
        if slice.lead {
            y += setter.measure(block, width).lead;
        }
        if let Block::Image { path, alt } = block {
            let drawn = plate_size(shelf.plates.get(path), width, height - PLATE_GAP * 2.0);
            let band = Rect::from_min_size(
                egui::pos2(rect.left() + (width - drawn.x).max(0.0) / 2.0, y + PLATE_GAP),
                drawn,
            );
            plate(&painter, &ctx, band, path, alt, shelf, books, plates, setting);
            y += drawn.y + PLATE_GAP * 2.0;
            continue;
        }
        let galley = ctx.fonts_mut(|f| f.layout_job(job(block, width, setting)));
        let rows = &galley.rows;
        let top = rows[slice.first_row].pos.y;
        let bottom = match rows.get(slice.first_row + slice.rows) {
            Some(next) => next.pos.y,
            None => galley.rect.height(),
        };
        let band = Rect::from_min_size(
            egui::pos2(rect.left(), y),
            Vec2::new(width, (bottom - top).max(0.0)),
        );
        // Clipped to its own band: a galley holds the whole block, and the rows this page did
        // not take must not be drawn over the next block's.
        let x = match block {
            Block::Heading { .. } => rect.left() + (width - galley.rect.width()).max(0.0) / 2.0,
            other => rect.left() + inset(other),
        };
        painter
            .with_clip_rect(band)
            .galley(egui::pos2(x, band.top() - top), galley, INK);
        y += band.height();
    }
    folio(ui, rect, *spine, shelf, setting, folio_number, counted.pages);
}

/// The place a saved offset resumes at, and the end of the chapter when a page was turned back
/// into it.
fn start_cursor<M: Measure>(doc: &Document, measure: &M, frame: PageFrame, offset: usize) -> Cursor {
    if offset == usize::MAX {
        return Cursor { block: doc.blocks.len(), row: 0 };
    }
    paginate::cursor_at(doc, measure, frame, offset)
}

/// One plate, decoded on its way to the screen and kept for a while afterwards.
///
/// The pixels live in the book's own zip, so they cannot come through the asset server — but
/// they also cannot be decoded during pagination, which runs over a shape and nothing else.
/// This is the only place a picture is turned into a picture.
#[allow(clippy::too_many_arguments)]
fn plate(
    painter: &egui::Painter,
    ctx: &egui::Context,
    band: Rect,
    path: &str,
    alt: &str,
    shelf: &Shelf,
    books: &mut Assets<Book>,
    plates: &mut Plates,
    setting: &Setting,
) {
    match plates.get(ctx, path, shelf, books) {
        Some(texture) => {
            painter.image(
                texture.id(),
                band,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        // A plate that will not decode still occupies the space the page was laid out around,
        // and says what it was supposed to be.
        None => {
            painter.rect_stroke(
                band,
                CornerRadius::same(2),
                Stroke::new(1.0_f32, RULE),
                egui::StrokeKind::Inside,
            );
            let words = if alt.is_empty() { "plate".to_owned() } else { alt.to_owned() };
            painter.text(
                band.center(),
                egui::Align2::CENTER_CENTER,
                words,
                setting.small.clone(),
                FAINT,
            );
        }
    }
}

/// Decoded plates, kept while they are worth keeping.
///
/// Bounded by count rather than by bytes because a plate is uploaded at a bounded size, and
/// cleared whole when the book changes — every key in it is a path inside one archive.
#[derive(Default)]
pub struct Plates {
    book: Option<String>,
    /// `None` records a plate that would not decode, so it is not read again every frame.
    held: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    order: std::collections::VecDeque<String>,
}

impl Plates {
    fn get(
        &mut self,
        ctx: &egui::Context,
        path: &str,
        shelf: &Shelf,
        books: &mut Assets<Book>,
    ) -> Option<egui::TextureHandle> {
        if self.book != shelf.file {
            self.book = shelf.file.clone();
            self.held.clear();
            self.order.clear();
        }
        if let Some(held) = self.held.get(path) {
            return held.clone();
        }
        let decoded = decode(ctx, path, shelf, books);
        self.held.insert(path.to_owned(), decoded.clone());
        self.order.push_back(path.to_owned());
        while self.order.len() > PLATE_CACHE {
            if let Some(old) = self.order.pop_front() {
                self.held.remove(&old);
            }
        }
        decoded
    }
}

fn decode(
    ctx: &egui::Context,
    path: &str,
    shelf: &Shelf,
    books: &mut Assets<Book>,
) -> Option<egui::TextureHandle> {
    let handle = shelf.handle.as_ref()?;
    let bytes = books.get_mut(handle)?.epub.resource(path).ok()?;
    let image = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let image = if image.width().max(image.height()) > PLATE_TEXELS {
        let scale = PLATE_TEXELS as f32 / image.width().max(image.height()) as f32;
        let (w, h) = (image.width() as f32 * scale, image.height() as f32 * scale);
        image.resize(w as u32, h as u32, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let colours = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(path, colours, egui::TextureOptions::LINEAR))
}

/// The foot of the page: where this is, in the two units that mean anything.
fn folio(
    ui: &mut egui::Ui,
    rect: Rect,
    spine: usize,
    shelf: &Shelf,
    setting: &Setting,
    number: usize,
    of: usize,
) {
    let painter = ui.painter();
    let y = rect.bottom() + 8.0;
    painter.line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        Stroke::new(1.0_f32, RULE),
    );
    painter.text(
        egui::pos2(rect.left(), y + 5.0),
        egui::Align2::LEFT_TOP,
        format!("chapter {} of {}", spine + 1, shelf.spine_count.max(1)),
        setting.small.clone(),
        FAINT,
    );
    painter.text(
        egui::pos2(rect.right(), y + 5.0),
        egui::Align2::RIGHT_TOP,
        // Of this chapter, at this size. The page number is the one thing on the screen that
        // is not written down anywhere, because it stops being true when the window moves.
        format!("page {number} of {}", of.max(1)),
        setting.small.clone(),
        FAINT,
    );
}

fn waiting(ui: &mut egui::Ui, shelf: &Shelf, setting: &Setting) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        let words = match &shelf.trouble {
            Some(why) => why.clone(),
            None => "opening the book".to_owned(),
        };
        ui.label(egui::RichText::new(words).color(FAINT).font(setting.body.clone()));
    });
}

/// The chapter list, on the page rather than in a window of its own.
fn contents(
    ui: &mut egui::Ui,
    shelf: &Shelf,
    books: &mut Assets<Book>,
    setting: &Setting,
    out: &mut MessageWriter<Requested>,
) {
    ui.label(egui::RichText::new("CONTENTS").size(10.0).color(FAINT));
    ui.add_space(10.0);
    let mut jump = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.visuals_mut().override_text_color = Some(INK);
        ui.visuals_mut().widgets.hovered.weak_bg_fill = RULE;
        ui.visuals_mut().widgets.active.weak_bg_fill = RULE;
        for entry in &shelf.chapters {
            let indent = 14.0 * entry.depth as f32;
            ui.horizontal(|ui| {
                ui.add_space(indent);
                let label = egui::RichText::new(&entry.label).font(setting.body.clone()).color(INK);
                if ui.add(egui::Button::new(label).fill(PAPER).frame(false)).clicked() {
                    jump = Some(entry.clone());
                }
            });
        }
    });
    if let Some(entry) = jump
        && let Some((spine, offset)) = crate::library::chapter_start(books, shelf, &entry)
    {
        ask(out, Action::GoTo(spine, offset));
    }
}
