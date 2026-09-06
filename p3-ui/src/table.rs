use p3_api::data::screen_rectangle::Rect;

use crate::{cell::Cell, page::Align, Page};

/// Room past a right-aligned column's digits that still counts as that column when a header
/// is clicked.
const HIT_SLACK: i32 = 6;
/// Between a sorted column's header and its sort marker.
const MARKER_GAP: i32 = 2;

/// One column: its window-relative `x` - the right edge of a right-aligned column, the left
/// edge otherwise - how cells sit against it, and how wide a cell is (rich-text wrap,
/// centring, and the header hit test).
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub x: i32,
    pub align: Align,
    pub width: i32,
}

impl Column {
    pub const fn right(x: i32, width: i32) -> Self {
        Column { x, align: Align::Right, width }
    }

    pub const fn left(x: i32, width: i32) -> Self {
        Column { x, align: Align::Left, width }
    }

    pub const fn center(x: i32, width: i32) -> Self {
        Column { x, align: Align::Center, width }
    }

    /// The window-relative span the column's cells cover, `[left, right)`.
    pub fn span(&self) -> (i32, i32) {
        match self.align {
            Align::Right => (self.x - self.width, self.x + HIT_SLACK),
            Align::Left | Align::Center => (self.x, self.x + self.width),
        }
    }
}

/// Rows of cells laid out in columns on a page.
pub struct Table<'p> {
    pub page: &'p Page,
    pub columns: Vec<Column>,
    /// The last y a row may start at; the page's unless [Table::ending_at] lowers it.
    pub last_y: i32,
}

impl<'p> Table<'p> {
    pub fn new(page: &'p Page, columns: impl Into<Vec<Column>>) -> Self {
        Table {
            page,
            columns: columns.into(),
            last_y: page.last_y,
        }
    }

    /// Stop the rows earlier than the page does.
    pub fn ending_at(mut self, last_y: i32) -> Self {
        self.last_y = last_y;
        self
    }

    pub fn fits(&self, y: i32) -> bool {
        y <= self.last_y
    }

    /// The header row in the heading face, one cell per column, with the sort marker (`^`
    /// natural, `v` reversed) right of column `sort.0`. Graphics are unaffected by the font.
    pub unsafe fn header(&self, y: i32, cells: &[Cell], sort: Option<(usize, bool)>) -> i32 {
        p3_api::ui::font::ddraw_set_font(p3_api::ui::font::get_header_font());
        self.draw_cells(y, cells);
        p3_api::ui::font::ddraw_set_font(p3_api::ui::font::get_normal_font());
        if let Some((index, reversed)) = sort {
            if let Some(column) = self.columns.get(index) {
                let marker: &[u8] = if reversed { b"v" } else { b"^" };
                let right = match column.align {
                    Align::Right => column.x,
                    Align::Left | Align::Center => column.x + column.width,
                };
                self.page.draw_text(right + MARKER_GAP, y, Align::Left, marker);
            }
        }
        y + self.page.row_height
    }

    /// One row, or nothing below [Table::last_y]; returns the next row's y.
    pub unsafe fn row(&self, y: i32, cells: &[Cell]) -> i32 {
        if !self.fits(y) {
            return y;
        }
        self.draw_cells(y, cells);
        y + self.page.row_height
    }

    /// [Table::row] with every cell in `color` instead of the page's black.
    pub unsafe fn row_colored(&self, y: i32, cells: &[Cell], color: u32) -> i32 {
        if !self.fits(y) {
            return y;
        }
        for (column, cell) in self.columns.iter().zip(cells) {
            if cell.is_empty() {
                continue;
            }
            let colored = Cell::Colored(color, Box::new(cell.clone()));
            self.page.draw_cell(column.x, y, column.align, column.width,&colored);
        }
        y + self.page.row_height
    }

    unsafe fn draw_cells(&self, y: i32, cells: &[Cell]) {
        for (column, cell) in self.columns.iter().zip(cells) {
            if cell.is_empty() {
                continue;
            }
            self.page.draw_cell(column.x, y, column.align, column.width,cell);
        }
    }

    /// The column whose header cell the screen point `x`, `y` is on, for a click on the
    /// header row at the window-relative `header_y`.
    pub fn hit_test(&self, x: i32, y: i32, header_y: i32) -> Option<usize> {
        let y = y - self.page.y;
        if y < header_y || y >= header_y + self.page.row_height {
            return None;
        }
        let x = x - self.page.x;
        self.columns.iter().position(|column| {
            let (left, right) = column.span();
            x >= left && x < right
        })
    }

    /// The window-relative extent of all columns, `[left, right)`.
    pub fn extent(&self) -> (i32, i32) {
        let left = self.columns.iter().map(|c| c.span().0).min().unwrap_or(0);
        let right = self.columns.iter().map(|c| c.span().1).max().unwrap_or(0);
        (left, right)
    }

    /// The screen rectangle of the rows between two window-relative y's (the header excluded
    /// when `top` is the first data row), spanning the columns; a scrollbar's wheel area.
    pub fn rows_area(&self, top: i32, bottom: i32) -> Rect {
        let (left, right) = self.extent();
        Rect {
            left: self.page.abs_x(left),
            top: self.page.abs_y(top),
            right: self.page.abs_x(right),
            bottom: self.page.abs_y(bottom),
        }
    }
}
