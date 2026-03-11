use esp_hal::{
    Blocking, gpio, peripherals,
    spi::master::{Config, Spi},
    time::Rate,
};

use crate::grid::{Column, Row};

pub struct ShiftRegister<'d> {
    spi: Spi<'d, Blocking>,
    cs: gpio::Output<'d>,
}

impl<'d> ShiftRegister<'d> {
    pub fn new(
        bus: peripherals::SPI2<'static>,
        cs: gpio::Output<'d>,
        sck: gpio::Output<'d>,
        miso: gpio::Input<'d>,
    ) -> Self {
        let spi = Spi::new(
            bus,
            Config::default()
                .with_frequency(Rate::from_khz(100))
                .with_mode(esp_hal::spi::Mode::_2),
        )
        .unwrap()
        .with_sck(sck)
        .with_miso(miso);
        // Manual CS control for inverted polarity
        Self { spi, cs }
    }

    // Device-specific methods
    pub async fn read(&mut self) -> Result<(Row, Column), ShiftRegisterError> {
        // Select device (CS high for active-high CS)
        self.cs.set_high();

        let mut rx_buf = [0u8; 5];
        self.spi
            .read(&mut rx_buf)
            .map_err(|e| ShiftRegisterError::SpiError(e))?;

        // Deselect device (CS low)
        self.cs.set_low();

        let value = rx_buf[4] as u64
            | (rx_buf[3] as u64) << 8
            | (rx_buf[2] as u64) << 16
            | (rx_buf[1] as u64) << 24
            | (rx_buf[0] as u64) << 32;

        // First 18 bits: bits 39-22 (MSB). Second 20 bits: bits 19-0 (LSB).
        // Scan each range MSB-first for first low (0).
        let column = (0..18)
            .find(|&i| (value >> (39 - i)) & 1 == 0)
            .map(|i| i as u8)
            .unwrap_or(0xFF);

        let row = (0..20)
            .find(|&i| (value >> (19 - i)) & 1 == 0)
            .map(|i| i as u8)
            .unwrap_or(0xFF);

        // Temporary issue because not all lines are pulled up
        let row = if row >= 6 { Row::IDLE } else { Row(row) };
        let column = if column >= 6 {
            Column::IDLE
        } else {
            Column(column)
        };

        Ok((row, column))
    }
}

#[derive(Debug)]
pub enum ShiftRegisterError {
    SpiError(esp_hal::spi::Error),
}
