//! App icon: red circle with white headphones, generated at
//! runtime (no files). BGRA pixels top to bottom, non-premultiplied alpha.

pub fn pixels(size: usize) -> Vec<u8> {
    let c = size as f32 / 2.0;
    let s = size as f32;
    let ss = 4;
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (mut acc, mut n) = ([0f32; 3], 0);
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f32 + (sx as f32 + 0.5) / ss as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / ss as f32;
                    if (px - c).hypot(py - c) > c - 0.5 {
                        continue;
                    }
                    let hy = c + s * 0.06;
                    let r = (px - c).hypot(py - hy);
                    let band = py < hy && r > s * 0.27 && r < s * 0.35;
                    let cup = py > s * 0.42
                        && py < s * 0.72
                        && ((px > s * 0.17 && px < s * 0.32) || (px > s * 0.68 && px < s * 0.83));
                    let col = if band || cup { [255.0, 255.0, 255.0] } else { [0x37 as f32, 0x18 as f32, 0xE3 as f32] };
                    for k in 0..3 {
                        acc[k] += col[k];
                    }
                    n += 1;
                }
            }
            let i = (y * size + x) * 4;
            if n > 0 {
                for k in 0..3 {
                    out[i + k] = (acc[k] / n as f32) as u8;
                }
                out[i + 3] = (255 * n / (ss * ss)) as u8;
            }
        }
    }
    out
}
