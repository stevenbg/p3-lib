//! Layout for the details pages: the text and tables a mod draws onto a building window's
//! spare page, and the detours that get it called.
//!
//! - [page::Page] is the window's geometry as rows: where the first row is, how tall a row
//!   is, and the last y a row may start at. It draws single cells and prose, and resets the
//!   renderer's colour, alignment and font to the page's defaults.
//! - [table::Table] lays [cell::Cell]s out in [table::Column]s - right or left aligned text,
//!   numbers, rich text with the game's inline symbols, or the game's graphics - and does the
//!   header, the hit test for header clicks, and the extents a scrollbar needs.
//! - [details_page_detours!] installs the two page-load detours and the open hook a page mod
//!   needs, verified against the bytes it replaces.
//! - [enlarge] makes a building window bigger than the 425 x 510 the game gives it, and keeps
//!   the shared backdrop behind it - picture, veil, frame - looking right at the new size.
//! - [graphics] holds drawing helpers composed from the game's blits.
//!
//! Everything is drawn in the game's own codepage (latin1); text arrives as bytes.

pub mod cell;
pub mod detours;
pub mod enlarge;
pub mod graphics;
pub mod page;
pub mod table;

pub use cell::{Cell, Symbol};
pub use page::{Align, Page, BLACK, ROW_HEIGHT};
pub use table::{Column, Table};

// Re-exported for the detour macro, so a page mod needs none of them as direct dependencies.
#[doc(hidden)]
pub use hooklet;
#[doc(hidden)]
pub use log;
#[doc(hidden)]
pub use p3_api;
