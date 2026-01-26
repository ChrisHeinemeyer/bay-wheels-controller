use int_enum::IntEnum;
use strum::EnumIter;

#[repr(u8)]
#[derive(Copy, Clone, IntEnum)]
pub enum ColorChannel {
    R = 0,
    G = 1,
    B = 2,
}

#[repr(u8)]
#[derive(Copy, Clone, IntEnum, EnumIter)]
pub enum Led {
    Led0 = 0,
    Led1 = 1,
    Led2 = 2,
    Led3 = 3,
    Led4 = 4,
    Led5 = 5,
    Led6 = 6,
    Led7 = 7,
    Led8 = 8,
    Led9 = 9,
    Led10 = 10,
    Led11 = 11,
}

#[derive(Copy, Clone)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}
