use p3_api::{
    data::{class48::Class48Ptr, screen_rectangle::Rect, ui_render_text_at},
    ui::{
        font::{self, TextMode},
        graphics::{draw_graphic_frame, graphic_frame_size},
        page_window::PageWindow,
        rect_clipper_stuff,
        rich_text::{draw_rich_text, font_escape},
    },
};

use crate::cell::Cell;

/// Opaque black, the pages' text colour.
pub const BLACK: u32 = 0xFF00_0000;
/// The row pitch every details page uses, so the pages look like one feature.
pub const ROW_HEIGHT: i32 = 16;
/// Width assumed for a graphic the game cannot measure.
const FALLBACK_GRAPHIC_WIDTH: i32 = 16;
/// Kept clear of the window frame on the right when prose is wrapped.
const PROSE_RIGHT_MARGIN: i32 = 12;

/// How a cell sits against its column's x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// x is the cell's left edge.
    Left,
    /// x is the cell's left edge and the cell is `width` wide; a graphic is centred in it by
    /// its measured width, text through the game's centre text mode.
    Center,
    /// x is the cell's right edge - the game's own alignment for figures.
    Right,
}

impl Align {
    fn text_mode(self) -> TextMode {
        match self {
            Align::Left => TextMode::AlignLeft,
            Align::Center => TextMode::AlignCenter,
            Align::Right => TextMode::AlignRight,
        }
    }

    fn rich_escape(self) -> &'static str {
        match self {
            Align::Left => "\\l",
            Align::Center => "\\c",
            Align::Right => "\\r",
        }
    }
}

/// A window's spare page as rows. `x`/`y` are the window's screen position; every other x
/// a caller passes is **window-relative**, every y absolute.
#[derive(Clone, Copy, Debug)]
pub struct Page {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// The window's text-layout object, when it has one; rich cells need it and fall back to
    /// plain text without it.
    pub layout: Option<u32>,
    pub row_height: i32,
    /// The first row's y.
    pub top: i32,
    /// The last y a row may start at; rows asked for below it are not drawn.
    pub last_y: i32,
}

impl Page {
    /// The page of `window`, its first row `first_row_y` below the window's top edge and its
    /// last row the one that still fits above the bottom edge.
    pub fn new<W: PageWindow>(window: &W, first_row_y: i32) -> Self {
        let (x, y, width, height) = (window.x(), window.y(), window.width(), window.height());
        Page {
            x,
            y,
            width,
            height,
            layout: window.layout(),
            row_height: ROW_HEIGHT,
            top: y + first_row_y,
            last_y: y + height - ROW_HEIGHT,
        }
    }

    /// Keep the bottom `rows` rows free of table rows - for a hint line, or for a scrollbar
    /// that has to clear the window's close button.
    pub fn reserve_bottom_rows(mut self, rows: i32) -> Self {
        self.last_y -= rows * self.row_height;
        self
    }

    /// Whether a row starting at `y` is still on the page.
    pub fn fits(&self, y: i32) -> bool {
        y <= self.last_y
    }

    /// The window's last row, below anything [Page::reserve_bottom_rows] kept free.
    pub fn bottom_row_y(&self) -> i32 {
        self.y + self.height - self.row_height
    }

    /// Half a row: the blank between blocks.
    pub fn gap(&self) -> i32 {
        self.row_height / 2
    }

    pub fn abs_x(&self, x: i32) -> i32 {
        self.x + x
    }

    pub fn rect(&self) -> Rect {
        Rect {
            left: self.x,
            top: self.y,
            right: self.x + self.width,
            bottom: self.y + self.height,
        }
    }

    /// Submit the window's area to the renderer. Call it from the window's **update** method,
    /// the phase the game itself uses; from the draw method the art tears and the text
    /// flickers.
    pub unsafe fn invalidate(&self) {
        rect_clipper_stuff(&self.rect());
    }

    /// The renderer state every page draws in: black, right-aligned, the body font. Called
    /// at the start of a draw and after anything that changes the state.
    pub unsafe fn reset_state(&self) {
        p3_api::data::ddraw_set_constant_color(BLACK);
        font::ddraw_set_text_mode(TextMode::AlignRight);
        font::ddraw_set_font(font::get_normal_font());
    }

    /// Draw one cell at the absolute `x`, aligned by `align`; `width` is the cell's width,
    /// which bounds rich-text wrapping and centres graphics. Leaves the state as
    /// [Page::reset_state] does.
    pub unsafe fn draw_cell(&self, x: i32, y: i32, align: Align, width: i32, cell: &Cell) {
        self.draw_cell_colored(x, y, align, width, cell, BLACK);
    }

    unsafe fn draw_cell_colored(&self, x: i32, y: i32, align: Align, width: i32, cell: &Cell, color: u32) {
        match cell {
            Cell::Empty => {}
            Cell::Text(bytes) => {
                if color != BLACK {
                    p3_api::data::ddraw_set_constant_color(color);
                }
                self.draw_text(x, y, align, bytes);
                if color != BLACK {
                    p3_api::data::ddraw_set_constant_color(BLACK);
                }
            }
            Cell::Rich(markup) => match self.layout {
                Some(layout) => {
                    // The pass takes its font from the markup, not from the renderer, and
                    // would default to the heading face.
                    let mut text = font_escape(font::NORMAL_FONT_INDEX).into_bytes();
                    text.extend_from_slice(align.rich_escape().as_bytes());
                    text.extend_from_slice(markup);
                    text.push(0);
                    draw_rich_text(layout, &text, x, y, width.max(1), self.row_height, color);
                    self.reset_state();
                }
                None => self.draw_cell_colored(x, y, align, width, &Cell::text(markup), color),
            },
            Cell::Graphic { id, frame } => {
                let graphic_width = graphic_frame_size(*id, *frame).map(|(w, _)| w).unwrap_or(FALLBACK_GRAPHIC_WIDTH);
                let left = match align {
                    Align::Left => x,
                    Align::Center => x + (width - graphic_width) / 2,
                    Align::Right => x - graphic_width,
                };
                draw_graphic_frame(*id, *frame, left, y);
                // The blit leaves the constant colour white.
                p3_api::data::ddraw_set_constant_color(BLACK);
            }
            Cell::Colored(color, inner) => self.draw_cell_colored(x, y, align, width, inner, *color),
        }
    }

    /// Plain text at the absolute `x`, in the current colour and font, leaving the text
    /// mode right-aligned.
    pub unsafe fn draw_text(&self, x: i32, y: i32, align: Align, text: &[u8]) {
        let mut buffer = Vec::with_capacity(text.len() + 1);
        buffer.extend_from_slice(text);
        buffer.push(0);
        if align != Align::Right {
            font::ddraw_set_text_mode(align.text_mode());
        }
        ui_render_text_at(x, y, &buffer);
        if align != Align::Right {
            font::ddraw_set_text_mode(TextMode::AlignRight);
        }
    }

    /// One cell on a row of its own at the window-relative `x`; returns the next row's y,
    /// or `y` unchanged when the row is below the page.
    pub unsafe fn line(&self, x: i32, y: i32, align: Align, cell: &Cell) -> i32 {
        if !self.fits(y) {
            return y;
        }
        let width = match align {
            Align::Left | Align::Center => self.width - x - PROSE_RIGHT_MARGIN,
            Align::Right => x,
        };
        self.draw_cell(self.abs_x(x), y, align, width, cell);
        y + self.row_height
    }

    /// A section heading in the heading face, then back to the body font.
    pub unsafe fn heading(&self, x: i32, y: i32, align: Align, text: &[u8]) -> i32 {
        if !self.fits(y) {
            return y;
        }
        font::ddraw_set_font(font::get_header_font());
        self.draw_text(self.abs_x(x), y, align, text);
        font::ddraw_set_font(font::get_normal_font());
        y + self.row_height
    }

    /// A line of prose from the window-relative `x` to the right margin.
    pub unsafe fn prose(&self, x: i32, y: i32, cell: &Cell) -> i32 {
        self.line(x, y, Align::Left, cell)
    }
}

/// The y, relative to the window, from which the game's background pass veils the building
/// animation in flat white at alpha 160, with a 160 px ramp fading in above it
/// (`p3_api::data::class48::Class48Ptr::set_gradient_y`). Above the ramp the animation keeps
/// its colours and text is hard to read, so 0 veils the whole window. A page whose text
/// starts lower could raise it and keep more of the animation, but every details page uses
/// this value so the buildings look alike.
pub const DEFAULT_GRADIENT_Y: u16 = 0;

/// Undo the clipping the windows set up for their own pages, so text drawn anywhere on the
/// window shows, with the gradient at [DEFAULT_GRADIENT_Y]. Once per window open is enough.
pub unsafe fn prepare_drawing_state() {
    prepare_drawing_state_with_gradient(DEFAULT_GRADIENT_Y);
}

/// [prepare_drawing_state] with the gradient at `gradient_y` instead of the default.
pub unsafe fn prepare_drawing_state_with_gradient(gradient_y: u16) {
    let class48 = Class48Ptr::new();
    class48.set_ignore_below_gradient(0);
    class48.set_gradient_y(gradient_y);
}
