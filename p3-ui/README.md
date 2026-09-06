# p3-ui

What the mods do to the game's UI, built on `p3-api`'s plain view of it: the pages they
draw onto a building window's spare page, the detours that get a page called, and the
enlargement of a building window beyond the size the game gives it. `p3-api` stays the
game as it is - pointers, fields, vtable slots, its own functions wrapped one to one - and
everything that is our policy on top lives here. `mod-tavern-details`,
`mod-church-details`, `mod-trading-office-details`, `mod-town-hall-details`,
`mod-shipyard-details` and `mod-trading-qol` are built on it.

## Drawing

- `Page` is the window's geometry as rows: `Page::new(&window, first_row_y)` gives the
  first row's y, the row pitch (16 px on every details page) and the last y a row may start
  at; `reserve_bottom_rows(n)` keeps rows free for a hint line or a scrollbar. It draws
  single cells (`line`, `heading`, `prose`) and `reset_state` puts the renderer back to
  black, right-aligned, body font. Every x and y a page takes or returns is relative to the
  window's top-left corner, so a layout holds wherever the window opens (and an offset
  outside the window draws outside it); `abs_x`/`abs_y` give the screen coordinates when a
  game call needs them, and `rect` is the window's screen area.
- `Table` lays `Cell`s out in `Column`s. A column names its window-relative x - the right
  edge of a right-aligned column, the left edge otherwise - and a width, which bounds
  rich-text wrapping, centres graphics and defines the header hit test. `header` draws the
  heading row (text or graphics) with an optional sort marker, `row` one row or nothing past
  the table's end, `row_colored` the same in another colour, `hit_test` maps a click on the
  header row to a column, and `extent` gives the span a scrollbar's wheel area needs.
- `Cell` is plain text, a number with an optional suffix, rich-text markup, a game graphic
  (or one frame of a sheet), or any of those in another colour. `Cell::amount(value, Symbol)`
  appends the game's coin, load or barrel symbol; rich cells go through the window's own
  text-layout object in the body font, so a window without one (`PageWindow::LAYOUT_OFFSET`
  is `None`) draws them as plain text.
- `graphics::blit_tiled` repeats a texture over a rectangle larger than itself, where the
  game's blit would read past the texture.

All text is in the game's codepage (latin1) and arrives as bytes.

## Plumbing

`details_page_detours!` installs, for one window type implementing
`p3_api::ui::page_window::PageWindow`, the two page-load detours (draw and update method)
and the open hook, verifying the six bytes at each patch site first. It defines
`install_page_detours() -> Result<(), u32>` for the mod's `start()` and calls back
`on_open(window)`, `on_update(window, page)` and `on_draw(window, page)`. Invoke it once per
crate. Hooks a page needs beyond that - close, mouse events, a page switcher - stay in the
mod.

## Enlarging a window

Every building window is 425 x 510, and the building backdrop behind all of them - the
interior picture, the whitening veil, the chains and the wooden frame, one shared object -
is 451 x 537; neither scales on its own. `enlarge` makes a window bigger in four calls from
the mod's own hooks: `before_open(&window, w, h)` ahead of the game's open method, which
centres the window and lays its widgets out from the new size; `after_open(w, h)` and
`on_close()`, which grow the backdrop around its centre while the window is open and give
it back to the other buildings; and `install_backdrop_hooks()` once from `start()`, which
paints the picture scaled to the enlarged area after the backdrop's own draw, veils it the
way the game does, redraws the frame in full, and redirects the backdrop's veil blits so
they scale the paper texture instead of reading past it.

The backdrop is one object and those hooks are process-wide; the module's documentation
explains what happens when more than one mod enlarges a window.
