# p3-page

Layout for the details pages: what a mod draws onto a building window's spare page, and
the detours that get it called. `mod-tavern-details`, `mod-church-details` and
`mod-trading-office-details` are built on it.

## Drawing

- `Page` is the window's geometry as rows: `Page::new(&window, first_row_y)` gives the
  first row's y, the row pitch (16 px on every details page) and the last y a row may start
  at; `reserve_bottom_rows(n)` keeps rows free for a hint line or a scrollbar. It draws
  single cells (`line`, `heading`, `prose`) and `reset_state` puts the renderer back to
  black, right-aligned, body font.
- `Table` lays `Cell`s out in `Column`s. A column names its window-relative x - the right
  edge of a right-aligned column, the left edge otherwise - and a width, which bounds
  rich-text wrapping, centres graphics and defines the header hit test. `header` draws the
  heading row (text or graphics) with an optional sort marker, `row` one row or nothing past
  the table's end, `hit_test` maps a click on the header row to a column, and `extent` gives
  the span a scrollbar's wheel area needs.
- `Cell` is plain text, a number with an optional suffix, rich-text markup, a game graphic
  (or one frame of a sheet), or any of those in another colour. `Cell::amount(value, Symbol)`
  appends the game's coin, load or barrel symbol; rich cells go through the window's own
  text-layout object in the body font, so a window without one (`PageWindow::LAYOUT_OFFSET`
  is `None`) draws them as plain text.

All text is in the game's codepage (latin1) and arrives as bytes.

## Plumbing

`details_page_detours!` installs, for one window type implementing
`p3_api::ui::page_window::PageWindow`, the two page-load detours (draw and update method)
and the open hook, verifying the six bytes at each patch site first. It defines
`install_page_detours() -> Result<(), u32>` for the mod's `start()` and calls back
`on_open(window)`, `on_update(window, page)` and `on_draw(window, page)`. Invoke it once per
crate. Hooks a page needs beyond that - close, mouse events, a page switcher - stay in the
mod.
