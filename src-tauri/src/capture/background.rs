use image::RgbaImage;

use super::NormalizedRect;

pub fn restore_text_background(image: &mut RgbaImage, bounds: NormalizedRect) -> bool {
    let width = image.width();
    let height = image.height();
    let rect = bounds.denormalize(width, height);
    let x0 = rect.x.max(0) as u32;
    let y0 = rect.y.max(0) as u32;
    let x1 = (x0 + rect.width).min(width);
    let y1 = (y0 + rect.height).min(height);
    let expanded_x0 = x0.saturating_sub(2);
    let expanded_y0 = y0.saturating_sub(2);
    let expanded_x1 = (x1 + 2).min(width);
    let expanded_y1 = (y1 + 2).min(height);
    let ring_x0 = expanded_x0.saturating_sub(3);
    let ring_y0 = expanded_y0.saturating_sub(3);
    let ring_x1 = (expanded_x1 + 3).min(width);
    let ring_y1 = (expanded_y1 + 3).min(height);
    let mut samples = Vec::new();
    for y in ring_y0..ring_y1 {
        for x in ring_x0..ring_x1 {
            if x < expanded_x0 || x >= expanded_x1 || y < expanded_y0 || y >= expanded_y1 {
                samples.push(*image.get_pixel(x, y));
            }
        }
    }
    if samples.is_empty() {
        return true;
    }
    let mut channels = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for sample in &samples {
        for index in 0..4 {
            channels[index].push(sample[index]);
        }
    }
    for values in &mut channels {
        values.sort_unstable();
    }
    let median = image::Rgba([
        channels[0][channels[0].len() / 2],
        channels[1][channels[1].len() / 2],
        channels[2][channels[2].len() / 2],
        channels[3][channels[3].len() / 2],
    ]);
    let variance = samples
        .iter()
        .map(|sample| {
            (0..3)
                .map(|index| {
                    let delta = sample[index] as f32 - median[index] as f32;
                    delta * delta
                })
                .sum::<f32>()
                / 3.0
        })
        .sum::<f32>()
        / samples.len() as f32;
    for y in expanded_y0..expanded_y1 {
        for x in expanded_x0..expanded_x1 {
            if variance.sqrt() < 18.0 {
                image.put_pixel(x, y, median);
            } else {
                let left = *image.get_pixel(ring_x0, y.clamp(ring_y0, ring_y1.saturating_sub(1)));
                let right = *image.get_pixel(
                    ring_x1.saturating_sub(1),
                    y.clamp(ring_y0, ring_y1.saturating_sub(1)),
                );
                let t = (x - expanded_x0) as f32 / (expanded_x1 - expanded_x0).max(1) as f32;
                image.put_pixel(
                    x,
                    y,
                    image::Rgba(std::array::from_fn(|i| {
                        ((left[i] as f32) * (1.0 - t) + (right[i] as f32) * t) as u8
                    })),
                );
            }
        }
    }
    variance.sqrt() >= 18.0
}
