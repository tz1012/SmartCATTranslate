use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use image::RgbaImage;

use super::{background::restore_text_background, DecodedImage, TranslatedBlock};

const BUNDLED_FONT: &[u8] = include_bytes!("../../../tests/fixtures/fonts/NotoSans-Variable.ttf");

#[derive(Clone, Debug)]
pub struct RenderedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub warnings: Vec<String>,
}

pub struct RenderEngine;

impl RenderEngine {
    pub fn render(
        source: &DecodedImage,
        blocks: &[TranslatedBlock],
    ) -> Result<RenderedImage, RenderError> {
        let mut image = RgbaImage::from_raw(source.width, source.height, source.rgba.clone())
            .ok_or(RenderError::InvalidImage)?;
        let mut warnings = Vec::new();
        let mut fonts = FontSystem::new();
        fonts.db_mut().load_font_data(BUNDLED_FONT.to_vec());
        fonts.db_mut().load_system_fonts();
        let mut cache = SwashCache::new();
        for block in blocks.iter().filter(|block| block.visible) {
            if restore_text_background(&mut image, block.bounds) {
                warnings.push(format!("backgroundApproximation:{}", block.id));
            }
            draw_block(&mut image, &mut fonts, &mut cache, block, &mut warnings)?;
        }
        Ok(RenderedImage {
            width: source.width,
            height: source.height,
            rgba: image.into_raw(),
            warnings,
        })
    }
}

fn draw_block(
    image: &mut RgbaImage,
    fonts: &mut FontSystem,
    cache: &mut SwashCache,
    block: &TranslatedBlock,
    warnings: &mut Vec<String>,
) -> Result<(), RenderError> {
    let rect = block.bounds.denormalize(image.width(), image.height());
    let max_width = rect.width as f32;
    let max_height = rect.height as f32;
    let mut low = 8.0f32;
    let mut high = (max_height * 1.05).max(8.0);
    let mut selected = 8.0;
    for _ in 0..7 {
        let size = (low + high) / 2.0;
        let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.22));
        buffer.set_size(fonts, Some(max_width), Some(max_height));
        buffer.set_text(
            fonts,
            &block.translated_text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(fonts, false);
        let fits = buffer
            .layout_runs()
            .all(|run| run.line_top + run.line_height <= max_height + 0.5);
        if fits {
            selected = size;
            low = size;
        } else {
            high = size;
        }
    }
    if selected <= 8.01 {
        warnings.push(format!("textOverflow:{}", block.id));
    }
    let mut buffer = Buffer::new(fonts, Metrics::new(selected, selected * 1.22));
    buffer.set_size(fonts, Some(max_width), Some(max_height));
    buffer.set_text(
        fonts,
        &block.translated_text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(fonts, false);
    let origin_x = rect.x.max(0);
    let origin_y = rect.y.max(0);
    let right = (origin_x as u32 + rect.width).min(image.width());
    let bottom = (origin_y as u32 + rect.height).min(image.height());
    buffer.draw(
        fonts,
        cache,
        Color::rgb(20, 25, 32),
        |x, y, width, height, color| {
            for py in 0..height {
                for px in 0..width {
                    let tx = origin_x + x + px as i32;
                    let ty = origin_y + y + py as i32;
                    if tx < origin_x
                        || ty < origin_y
                        || tx < 0
                        || ty < 0
                        || tx as u32 >= right
                        || ty as u32 >= bottom
                    {
                        continue;
                    }
                    let alpha = color.a() as f32 / 255.0;
                    let pixel = image.get_pixel_mut(tx as u32, ty as u32);
                    for channel in 0..3 {
                        pixel[channel] = (color.as_rgba_tuple().0 as f32 * alpha
                            + pixel[channel] as f32 * (1.0 - alpha))
                            as u8;
                    }
                }
            }
        },
    );
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("invalid image buffer")]
    InvalidImage,
}
