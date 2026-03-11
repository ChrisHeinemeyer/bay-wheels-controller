//! Grid position types for the shift-register input matrix.
//!
//! Row and Column represent indices into the 6×6 (or larger) input grid.
//! Value 0xFF denotes idle (no input active).

/// Row index from the shift register. 0..6 = valid, 0xFF = idle.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Row(pub u8);

/// Column index from the shift register. 0..6 = valid, 0xFF = idle.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Column(pub u8);

impl Row {
    /// No input active.
    pub const IDLE: Self = Self(0xFF);
}

impl Column {
    /// No input active.
    pub const IDLE: Self = Self(0xFF);
}

impl From<Row> for u8 {
    fn from(r: Row) -> u8 {
        r.0
    }
}

impl From<Column> for u8 {
    fn from(c: Column) -> u8 {
        c.0
    }
}
