use esp_hal::{
    Blocking, gpio, peripherals,
    spi::master::{Config, Spi},
    time::Rate,
};

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
    pub async fn read(&mut self) -> Result<u16, ShiftRegisterError> {
        // Select device (CS high for active-high CS)
        self.cs.set_high();

        let mut rx_buf = [0u8; 2];
        self.spi
            .read(&mut rx_buf)
            .map_err(|e| ShiftRegisterError::SpiError(e))?;

        // Deselect device (CS low)
        self.cs.set_low();

        Ok(rx_buf[1] as u16 | (rx_buf[0] as u16) << 8)
    }
}

#[derive(Debug)]
pub enum ShiftRegisterError {
    SpiError(esp_hal::spi::Error),
}
