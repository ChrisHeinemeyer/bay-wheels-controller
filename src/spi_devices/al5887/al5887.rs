use heapless::Vec;
use embassy_time::{Duration, Timer};
use esp_hal::{
    Blocking, gpio, peripherals,
    spi::master::{Config, Spi},
    time::Rate,
};
use strum::IntoEnumIterator;


use crate::{spi_devices::al5887::enums::{Color, ColorChannel}, tasks::station_leds::MAX_LEDS};
use crate::spi_devices::al5887::{enums::Led, registers::Register};
#[derive(Copy, Clone)]
pub struct SpiFrame {
    register: Register,
    value: u8,
    write: bool,
}

impl SpiFrame {
    pub fn new(register: Register, value: u8, write: bool) -> Self {
        Self {
            register,
            value,
            write,
        }
    }

    pub fn to_bytes(&self) -> [u8; 2] {
        [(self.register as u8) << 1 | self.write as u8, self.value]
    }

    pub fn set_chip_enable(enable: bool) -> Self {
        let value = (enable as u8) << 6;
        Self::new(Register::DeviceConfig0, value, true)
    }

    pub fn set_led_brightness_color(led: Led, brightness: u8, color: Color) -> [SpiFrame; 4] {
        let brightness_register = Register::brightness_register(led);
        let color_register_r = Register::color_register(led, ColorChannel::R);
        let color_register_g = Register::color_register(led, ColorChannel::G);
        let color_register_b = Register::color_register(led, ColorChannel::B);
        [
            Self::new(brightness_register, brightness, true),
            Self::new(color_register_r, color.r, true),
            Self::new(color_register_g, color.g, true),
            Self::new(color_register_b, color.b, true),
        ]
    }
}
pub struct Al5887<'d> {
    spi: Spi<'d, Blocking>,
    led_en: gpio::Output<'d>,
    led_rst_n: gpio::Output<'d>,
}

impl<'d> Al5887<'d> {
    pub fn new(
        bus: peripherals::SPI3<'static>,
        cs: gpio::Output<'d>,
        sck: gpio::Output<'d>,
        miso: gpio::Input<'d>,
        mosi: gpio::Output<'d>,
        led_en: gpio::Output<'d>,
        led_rst_n: gpio::Output<'d>,
    ) -> Self {
        let spi = Spi::new(
            bus,
            Config::default()
                .with_frequency(Rate::from_khz(100))
                .with_mode(esp_hal::spi::Mode::_0),
        )
        .unwrap()
        .with_sck(sck)
        .with_cs(cs)
        .with_miso(miso)
        .with_mosi(mosi);
        // Manual CS control for inverted polarity
        Self {
            spi,
            led_en,
            led_rst_n,
        }
    }

    pub async fn read_register(&mut self, spi_frame: SpiFrame) -> Result<u8, Al5887Error> {
        let mut rx_buf = [0u8; 1];
        self.spi
            .write(&spi_frame.to_bytes())
            .map_err(|e| Al5887Error::SpiError(e))?;
        self.spi
            .read(&mut rx_buf)
            .map_err(|e| Al5887Error::SpiError(e))?;
        Ok(rx_buf[0])
    }

    pub async fn write_register(&mut self, spi_frame: SpiFrame) -> Result<(), Al5887Error> {
        self.spi
            .write(&spi_frame.to_bytes())
            .map_err(|e| Al5887Error::SpiError(e))?;
        Ok(())
    }

    pub async fn set_chip_enable(&mut self, enable: bool) -> Result<(), Al5887Error> {
        self.write_register(SpiFrame::set_chip_enable(enable))
            .await?;
        Ok(())
    }

    pub async fn init_driver(&mut self) -> Result<(), Al5887Error> {
        self.led_en.set_high();
        self.led_rst_n.set_low();
        Timer::after(Duration::from_millis(1)).await;
        self.led_rst_n.set_high();
        self.set_chip_enable(true).await?;
        let led_0_brightness_color =
            SpiFrame::set_led_brightness_color(Led::Led0, 127, Color::new(255, 255, 255));
        self.write_register(led_0_brightness_color[0]).await?;
        self.write_register(led_0_brightness_color[1]).await?;
        self.write_register(led_0_brightness_color[2]).await?;
        self.write_register(led_0_brightness_color[3]).await?;
        Ok(())
    }

    pub async fn set_led_brightness_color(
        &mut self,
        led: Led,
        brightness: u8,
        color: Color,
    ) -> Result<(), Al5887Error> {
        // Because of a wiring but, we need to write the color in the opposite order
        let color = Color::new(color.b, color.g, color.r);
        let spi_frames = SpiFrame::set_led_brightness_color(led, brightness, color);
        self.write_register(spi_frames[0]).await?;
        self.write_register(spi_frames[1]).await?;
        self.write_register(spi_frames[2]).await?;
        self.write_register(spi_frames[3]).await?;
        Ok(())
    }

    pub async fn set_all_leds_brightness_color(
        &mut self,
        brightness: u8,
        color: Color,
    ) -> Result<(), Al5887Error> {
        for led in Led::iter() {
            self.set_led_brightness_color(led, brightness, color)
                .await?;
        }
        Ok(())
    }

    pub async fn set_vec_led(&mut self, data: Vec<(Led, Color), MAX_LEDS>) -> Result<(), Al5887Error> {
        for data_item in data.iter() {
            self.set_led_brightness_color(data_item.0, 30, data_item.1)
                .await?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum Al5887Error {
    SpiError(esp_hal::spi::Error),
    InvalidResponse(u8),
}
