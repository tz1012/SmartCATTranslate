use uuid::Uuid;

use super::{NormalizedRect, OcrDocument, OcrLine, TextDirection};

#[derive(Clone, Debug)]
pub struct TextBlock {
    pub id: Uuid,
    pub source_ids: Vec<Uuid>,
    pub text: String,
    pub bounds: NormalizedRect,
    pub confidence: f32,
    pub direction: TextDirection,
}

pub fn group_lines(document: &OcrDocument) -> Vec<TextBlock> {
    let mut lines = document.lines.iter().collect::<Vec<_>>();
    lines.sort_by(|a, b| {
        a.bounds
            .x
            .total_cmp(&b.bounds.x)
            .then(a.bounds.y.total_cmp(&b.bounds.y))
    });
    let mut columns: Vec<Vec<&OcrLine>> = Vec::new();
    for line in lines {
        let center = line.bounds.x + line.bounds.width / 2.0;
        if let Some(column) = columns.iter_mut().find(|column| {
            let sample = column[0];
            center >= sample.bounds.x - 0.03
                && center <= sample.bounds.x + sample.bounds.width + 0.03
        }) {
            column.push(line);
        } else {
            columns.push(vec![line]);
        }
    }
    columns.sort_by(|a, b| a[0].bounds.x.total_cmp(&b[0].bounds.x));
    let mut result = Vec::new();
    for mut column in columns {
        column.sort_by(|a, b| {
            a.bounds
                .y
                .total_cmp(&b.bounds.y)
                .then(a.bounds.x.total_cmp(&b.bounds.x))
        });
        let mut current: Vec<&OcrLine> = Vec::new();
        for line in column {
            let joins = current
                .last()
                .is_some_and(|previous| can_join(previous, line));
            if !joins && !current.is_empty() {
                result.push(block_from(&current));
                current.clear();
            }
            current.push(line);
        }
        if !current.is_empty() {
            result.push(block_from(&current));
        }
    }
    result
}

fn can_join(a: &OcrLine, b: &OcrLine) -> bool {
    let angle = (a.angle_degrees - b.angle_degrees).abs() <= 3.0;
    let vertical_gap = (b.bounds.y - (a.bounds.y + a.bounds.height)).max(0.0);
    let gap = vertical_gap <= a.bounds.height.min(b.bounds.height) * 0.8;
    let overlap =
        (a.bounds.x + a.bounds.width).min(b.bounds.x + b.bounds.width) - a.bounds.x.max(b.bounds.x);
    let horizontal = overlap > a.bounds.width.min(b.bounds.width) * 0.25;
    angle && gap && horizontal && a.direction == b.direction
}

fn block_from(lines: &[&OcrLine]) -> TextBlock {
    let left = lines.iter().map(|line| line.bounds.x).fold(1.0, f32::min);
    let top = lines.iter().map(|line| line.bounds.y).fold(1.0, f32::min);
    let right = lines
        .iter()
        .map(|line| line.bounds.x + line.bounds.width)
        .fold(0.0, f32::max);
    let bottom = lines
        .iter()
        .map(|line| line.bounds.y + line.bounds.height)
        .fold(0.0, f32::max);
    TextBlock {
        id: Uuid::new_v4(),
        source_ids: lines.iter().map(|line| line.id).collect(),
        text: lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        bounds: NormalizedRect::new(left, top, right - left, bottom - top)
            .expect("OCR bounds were validated"),
        confidence: lines.iter().map(|line| line.confidence).fold(1.0, f32::min),
        direction: lines[0].direction,
    }
}
