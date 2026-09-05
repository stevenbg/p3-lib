//! Drawing helpers composed from the game's blits.

use p3_api::ui::graphics::blit;

/// [blit] a rectangle that may be larger than the selected texture, repeating the texture
/// instead of reading past it: the source wraps at `texture_width` x `texture_height`. A
/// plain blit with a source rectangle beyond the texture reads off the end of its buffer
/// and crashes in `ddraw_dll`.
pub unsafe fn blit_tiled(texture_width: i32, texture_height: i32, source_x: i32, source_y: i32, x: i32, y: i32, width: i32, height: i32) {
    if texture_width <= 0 || texture_height <= 0 {
        return;
    }
    let mut drawn_y = 0;
    while drawn_y < height {
        let src_y = (source_y + drawn_y) % texture_height;
        let rows = (texture_height - src_y).min(height - drawn_y);
        let mut drawn_x = 0;
        while drawn_x < width {
            let src_x = (source_x + drawn_x) % texture_width;
            let columns = (texture_width - src_x).min(width - drawn_x);
            blit(src_x, src_y, x + drawn_x, y + drawn_y, columns, rows);
            drawn_x += columns;
        }
        drawn_y += rows;
    }
}
