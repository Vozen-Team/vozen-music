use fontdue::{
    Font, FontSettings,
    layout::{CoordinateSystem, HorizontalAlign, Layout, LayoutSettings, TextStyle},
};
use image::{ImageFormat, Rgb, RgbImage, imageops::FilterType};
use std::{io::Cursor, path::Path, sync::OnceLock};
use vozen_music::{Error, Track};

const WIDTH: u32 = 768;
const BACKGROUND: Rgb<u8> = Rgb([35, 36, 40]);

fn fonts() -> &'static [Font; 2] {
    static FONTS: OnceLock<[Font; 2]> = OnceLock::new();
    FONTS.get_or_init(|| {
        [
            Font::from_bytes(
                include_bytes!("../assets/fonts/Outfit-SemiBold.ttf") as &[u8],
                FontSettings::default(),
            )
            .expect("validated bundled Outfit SemiBold"),
            Font::from_bytes(
                include_bytes!("../assets/fonts/Outfit-Regular.ttf") as &[u8],
                FontSettings::default(),
            )
            .expect("validated bundled Outfit Regular"),
        ]
    })
}

pub fn filename(cover: &Path) -> String {
    format!(
        "{}-player-v1.png",
        cover.file_stem().unwrap_or_default().to_string_lossy()
    )
}

fn text_layout(text: &str, size: f32, font: usize, y: f32) -> Layout {
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: 32.0,
        y,
        max_width: Some((WIDTH - 64) as f32),
        horizontal_align: HorizontalAlign::Center,
        ..LayoutSettings::default()
    });
    layout.append(fonts(), &TextStyle::new(text, size, font));
    layout
}

fn draw_text(canvas: &mut RgbImage, layout: &Layout, color: [u8; 3]) -> Result<(), Error> {
    for glyph in layout.glyphs() {
        let (metrics, bitmap) = fonts()[glyph.font_index].rasterize_config(glyph.key);
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let alpha = bitmap[row * metrics.width + col] as u32;
                if alpha == 0 {
                    continue;
                }
                let x = glyph.x.floor() as i32 + col as i32;
                let y = glyph.y.floor() as i32 + row as i32;
                if x < 0 || y < 0 || x >= canvas.width() as i32 || y >= canvas.height() as i32 {
                    return Err("Recording text does not fit the player artwork.".into());
                }
                let pixel = canvas.get_pixel_mut(x as u32, y as u32);
                for channel in 0..3 {
                    pixel[channel] = ((color[channel] as u32 * alpha
                        + pixel[channel] as u32 * (255 - alpha)
                        + 127)
                        / 255) as u8;
                }
            }
        }
    }
    Ok(())
}

pub fn render(cover: &[u8], title: &str, artist: &str) -> Result<Vec<u8>, Error> {
    let cover = image::load_from_memory_with_format(cover, ImageFormat::Png)?
        .resize_exact(WIDTH, WIDTH, FilterType::Lanczos3)
        .to_rgb8();
    let title = text_layout(title, 54.0, 0, WIDTH as f32 + 26.0);
    let artist = text_layout(artist, 35.0, 1, WIDTH as f32 + 26.0 + title.height() + 8.0);
    let height =
        (WIDTH as f32 + 26.0 + title.height() + 8.0 + artist.height() + 26.0).ceil() as u32;
    let mut canvas = RgbImage::from_pixel(WIDTH, height, BACKGROUND);
    image::imageops::replace(&mut canvas, &cover, 0, 0);
    // Rounded artwork corners share the caption background; there is no second frame.
    let radius = 22.0_f32;
    for y in 0..WIDTH {
        for x in 0..WIDTH {
            let dx = (radius - (x as f32 + 0.5).min(WIDTH as f32 - x as f32 - 0.5)).max(0.0);
            let dy = (radius - (y as f32 + 0.5).min(WIDTH as f32 - y as f32 - 0.5)).max(0.0);
            if dx == 0.0 || dy == 0.0 {
                continue;
            }
            let opacity = (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
            let pixel = canvas.get_pixel_mut(x, y);
            for c in 0..3 {
                pixel[c] = (pixel[c] as f32 * opacity + BACKGROUND[c] as f32 * (1.0 - opacity))
                    .round() as u8;
            }
        }
    }
    draw_text(&mut canvas, &title, [246, 246, 248])?;
    draw_text(&mut canvas, &artist, [173, 177, 187])?;
    let mut png = Cursor::new(Vec::new());
    canvas.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}

pub async fn attachment(track: &Track) -> Result<serenity::all::CreateAttachment, Error> {
    let file = track
        .cover
        .as_ref()
        .ok_or("Recording artwork is unavailable.")?;
    let name = filename(file);
    let cover = tokio::fs::read(file).await?;
    let title = track.title.clone();
    let artist = track.artist.clone();
    let png = tokio::task::spawn_blocking(move || render(&cover, &title, &artist)).await??;
    Ok(serenity::all::CreateAttachment::bytes(png, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_recordings_have_supported_centered_text_and_dreams_matches_the_approved_composition() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let catalog = vozen_music::Catalog::load(root).unwrap();
        for track in catalog.stations.iter().flat_map(|s| &s.tracks) {
            for (text, font, size) in [(&track.title, 0, 54.0), (&track.artist, 1, 35.0)] {
                assert!(
                    text.chars()
                        .filter(|c| !c.is_whitespace())
                        .all(|c| fonts()[font].lookup_glyph_index(c) != 0),
                    "Missing glyph for {text}"
                );
                let layout = text_layout(text, size, font, 0.0);
                assert!(
                    layout.height() > 0.0 && layout.height() < 360.0,
                    "Text is too tall: {text}"
                );
                for glyph in layout.glyphs().iter().filter(|g| g.width > 0) {
                    assert!(
                        glyph.x >= 20.0 && glyph.x + glyph.width as f32 <= WIDTH as f32 - 20.0,
                        "Text exceeds margins: {text}"
                    );
                }
            }
        }
        let track = catalog
            .stations
            .iter()
            .flat_map(|s| &s.tracks)
            .find(|t| t.title == "Dreams")
            .unwrap();
        let cover = std::fs::read(track.cover.as_ref().unwrap()).unwrap();
        let png = render(&cover, &track.title, &track.artist).unwrap();
        let card = image::load_from_memory(&png).unwrap().to_rgb8();
        assert_eq!(card.width(), WIDTH);
        assert!(card.height() > WIDTH && card.height() < WIDTH + 240);
        assert_eq!(*card.get_pixel(0, 0), BACKGROUND);
        assert_eq!(*card.get_pixel(0, WIDTH + 12), BACKGROUND);
        assert_eq!(*card.get_pixel(WIDTH - 1, card.height() - 1), BACKGROUND);
        let title_pixels: Vec<_> = (0..WIDTH)
            .filter(|x| (WIDTH + 26..WIDTH + 80).any(|y| *card.get_pixel(*x, y) != BACKGROUND))
            .collect();
        let center = (title_pixels.first().unwrap() + title_pixels.last().unwrap()) as f32 / 2.0;
        assert!(
            (center - WIDTH as f32 / 2.0).abs() < 4.0,
            "Title is not visually centered"
        );
        assert!(render(b"invalid PNG", "Dreams", "HoliznaCC0").is_err());
        std::fs::create_dir_all(root.join("output")).unwrap();
        std::fs::write(root.join("output/player-card-dreams.png"), png).unwrap();
        let longest = catalog
            .stations
            .iter()
            .flat_map(|s| &s.tracks)
            .max_by_key(|t| t.title.len())
            .unwrap();
        let png = render(&cover, &longest.title, &longest.artist).unwrap();
        std::fs::write(root.join("output/player-card-long-title.png"), png).unwrap();
    }
}
