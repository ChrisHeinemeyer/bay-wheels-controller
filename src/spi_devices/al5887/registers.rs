use int_enum::IntEnum;

use crate::spi_devices::al5887::enums::{ColorChannel, Led};

#[repr(u8)]
#[derive(Copy, Clone, IntEnum)]
pub enum Register {
    DeviceConfig0 = 0x00,
    DeviceConfig1 = 0x01,
    LedConfig0 = 0x02,
    LedConfig1 = 0x03,
    BankBrightness = 0x04,
    BankAColor = 0x05,
    BankBColor = 0x06,
    BankCColor = 0x07,
    Rgb0Brightness = 0x08,
    Rgb1Brightness = 0x09,
    Rgb2Brightness = 0x0A,
    Rgb3Brightness = 0x0B,
    Rgb4Brightness = 0x0C,
    Rgb5Brightness = 0x0D,
    Rgb6Brightness = 0x0E,
    Rgb7Brightness = 0x0F,
    Rgb8Brightness = 0x10,
    Rgb9Brightness = 0x11,
    Rgb10Brightness = 0x12,
    Rgb11Brightness = 0x13,
    R0Color = 0x14,
    G0Color = 0x15,
    B0Color = 0x16,
    R1Color = 0x17,
    G1Color = 0x18,
    B1Color = 0x19,
    R2Color = 0x1A,
    G2Color = 0x1B,
    B2Color = 0x1C,
    R3Color = 0x1D,
    G3Color = 0x1E,
    B3Color = 0x1F,
    R4Color = 0x20,
    G4Color = 0x21,
    B4Color = 0x22,
    R5Color = 0x23,
    G5Color = 0x24,
    B5Color = 0x25,
    R6Color = 0x26,
    G6Color = 0x27,
    B6Color = 0x28,
    R7Color = 0x29,
    G7Color = 0x2A,
    B7Color = 0x2B,
    R8Color = 0x2C,
    G8Color = 0x2D,
    B8Color = 0x2E,
    R9Color = 0x2F,
    G9Color = 0x30,
    B9Color = 0x31,
    R10Color = 0x32,
    G10Color = 0x33,
    B10Color = 0x34,
    R11Color = 0x35,
    G11Color = 0x36,
    B11Color = 0x37,
    Reset = 0x38,
    Flag = 0x65,
    LedGlobalDimming = 0x66,
    FaultWait = 0x67,
    MaskAndClr = 0x68,
}

impl Register {
    pub fn brightness_register(led: Led) -> Self {
        // add 8 to the register
        let register = { Register::Rgb0Brightness as u8 + led as u8 };
        Self::try_from(register).unwrap()
    }

    pub fn color_register(led: Led, channel: ColorChannel) -> Self {
        let register = { Register::R0Color as u8 + (3 * led as u8) + channel as u8 };
        Self::try_from(register).unwrap()
    }
}
