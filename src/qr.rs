// src/qr.rs
use qrcode::{QrCode, EcLevel};
use image::{ImageBuffer, Rgb, RgbImage};
use base64::{Engine as _, engine::general_purpose};

#[derive(Debug, Clone)]
pub struct QrService;

impl QrService {
    pub fn new() -> Self {
        Self
    }

    /// Генерировать QR код в формате base64 data URL
    pub fn generate_qr_code(&self, data: &str) -> anyhow::Result<String> {
        // Создаем QR код
        let code = QrCode::with_error_correction_level(data, EcLevel::M)?;

        // Настройки изображения
        let size = 10; // Размер пикселя
        let border = 4; // Размер рамки

        // Размеры
        let width = code.width();
        let img_size = (width + 2 * border) * size;

        // Создаем изображение
        let mut img: RgbImage = ImageBuffer::new(img_size as u32, img_size as u32);

        // Заполняем белым фоном
        for pixel in img.pixels_mut() {
            *pixel = Rgb([255, 255, 255]);
        }

        // Рисуем QR код
        for y in 0..width {
            for x in 0..width {
                if code[(x, y)] == qrcode::Color::Dark {
                    // Рисуем черный квадрат
                    for dy in 0..size {
                        for dx in 0..size {
                            let px = (border + x) * size + dx;
                            let py = (border + y) * size + dy;
                            if px < img_size && py < img_size {
                                img.put_pixel(px as u32, py as u32, Rgb([0, 0, 0]));
                            }
                        }
                    }
                }
            }
        }

        // Конвертируем в PNG bytes
        let mut png_bytes = Vec::new();
        {
            use image::codecs::png::PngEncoder;
            use image::ImageEncoder;

            let encoder = PngEncoder::new(&mut png_bytes);
            encoder.write_image(
                img.as_raw(),
                img_size as u32,
                img_size as u32,
                image::ColorType::Rgb8,
            )?;
        }

        // Кодируем в base64
        let base64_string = general_purpose::STANDARD.encode(&png_bytes);

        Ok(format!("data:image/png;base64,{}", base64_string))
    }

