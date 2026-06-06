use tray_icon::Icon;

pub fn circle_icon(rgb: [u8; 3]) -> Icon {
    const S: u32 = 22;
    let mut rgba = vec![0u8; (S * S * 4) as usize];
    let c = S as f32 / 2.0;
    let r = c - 2.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let a = ((r - (dx * dx + dy * dy).sqrt() + 1.0).clamp(0.0, 1.0) * 255.0) as u8;
            let i = ((y * S + x) * 4) as usize;
            rgba[i] = rgb[0]; rgba[i + 1] = rgb[1]; rgba[i + 2] = rgb[2]; rgba[i + 3] = a;
        }
    }
    Icon::from_rgba(rgba, S, S).unwrap()
}
