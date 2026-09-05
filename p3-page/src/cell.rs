use std::{borrow::Cow, ffi::CStr};

/// One of the inline symbols the game's rich-text pass draws in place of an escape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    /// `\C`, the coin.
    Coin,
    /// `\L`, a load of cargo.
    Load,
    /// `\B`, a barrel.
    Barrel,
}

impl Symbol {
    pub const fn escape(self) -> &'static str {
        match self {
            Symbol::Coin => "\\C",
            Symbol::Load => "\\L",
            Symbol::Barrel => "\\B",
        }
    }
}

/// What one cell of a row holds.
#[derive(Clone, Debug)]
pub enum Cell<'a> {
    Empty,
    /// Plain text in the game's codepage.
    Text(Cow<'a, [u8]>),
    /// Rich-text markup - inline [Symbol]s, tabs - drawn through the window's text-layout
    /// object. Alignment and the body font are added by the page; do not start the markup
    /// with an escape of your own.
    Rich(Vec<u8>),
    /// One of the game's graphics, or one frame of a sheet, placed by its measured width.
    Graphic { id: u32, frame: u32 },
    /// The inner cell in a colour other than the page's black.
    Colored(u32, Box<Cell<'a>>),
}

impl<'a> Cell<'a> {
    pub fn text(bytes: &'a [u8]) -> Self {
        Cell::Text(Cow::Borrowed(bytes))
    }

    pub fn owned(bytes: Vec<u8>) -> Self {
        Cell::Text(Cow::Owned(bytes))
    }

    pub fn number(value: i32) -> Self {
        Cell::owned(value.to_string().into_bytes())
    }

    /// `value` followed by `suffix`, e.g. `" %"`.
    pub fn number_with(value: i32, suffix: &str) -> Self {
        Cell::owned(format!("{value}{suffix}").into_bytes())
    }

    /// `value` followed by the game's symbol for it: `3333` and a coin, `12` and a load.
    pub fn amount(value: i32, symbol: Symbol) -> Self {
        Cell::Rich(format!("{value}{}", symbol.escape()).into_bytes())
    }

    pub fn rich(markup: impl Into<Vec<u8>>) -> Self {
        Cell::Rich(markup.into())
    }

    pub fn graphic(id: u32) -> Self {
        Cell::Graphic { id, frame: 0 }
    }

    pub fn graphic_frame(id: u32, frame: u32) -> Self {
        Cell::Graphic { id, frame }
    }

    pub fn colored(self, color: u32) -> Self {
        Cell::Colored(color, Box::new(self))
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Cell::Empty => true,
            Cell::Text(bytes) => bytes.is_empty(),
            Cell::Rich(markup) => markup.is_empty(),
            Cell::Graphic { .. } => false,
            Cell::Colored(_, inner) => inner.is_empty(),
        }
    }
}

impl<'a> From<&'a [u8]> for Cell<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Cell::text(bytes)
    }
}

impl<'a> From<&'a CStr> for Cell<'a> {
    fn from(text: &'a CStr) -> Self {
        Cell::text(text.to_bytes())
    }
}

impl<'a> From<&'a str> for Cell<'a> {
    fn from(text: &'a str) -> Self {
        Cell::text(text.as_bytes())
    }
}

impl From<String> for Cell<'_> {
    fn from(text: String) -> Self {
        Cell::owned(text.into_bytes())
    }
}

impl From<Vec<u8>> for Cell<'_> {
    fn from(bytes: Vec<u8>) -> Self {
        Cell::owned(bytes)
    }
}

impl From<i32> for Cell<'_> {
    fn from(value: i32) -> Self {
        Cell::number(value)
    }
}
