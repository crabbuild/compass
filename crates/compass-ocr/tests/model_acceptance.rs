use compass_ocr::{
    ManagedOarEngine, ModelProfile, OCR_SCHEMA, OcrEngine, OcrRequest, OcrSourceKind,
    PreparedRaster, prepared_raster_digest,
};
use image::{Rgb, RgbImage};

#[test]
#[ignore = "requires an already-installed verified pp-ocrv6-small profile"]
fn installed_profile_runs_without_external_runtime_dependencies()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = match ManagedOarEngine::load(ModelProfile::PpOcrV6Small) {
        Ok(engine) => engine,
        Err(error) if error.to_string().contains("models install pp-ocrv6-small") => {
            eprintln!("SKIP: exact pp-ocrv6-small profile is not installed");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let image = RgbImage::from_pixel(256, 128, Rgb([255, 255, 255]));
    let raster = PreparedRaster {
        image,
        width: 256,
        height: 128,
    };
    let request = OcrRequest {
        schema: OCR_SCHEMA.to_owned(),
        request_id: "installed-model-acceptance".to_owned(),
        source_kind: OcrSourceKind::EmbeddedImage,
        width: 256,
        height: 128,
        language_hints: Vec::new(),
        image_digest: prepared_raster_digest(&raster),
    };
    let response = engine.recognize(&request, &raster)?;
    response.validate_for(&request)?;
    assert_eq!(response.profile.profile, "pp-ocrv6-small");

    let image = synthetic_clean_english("COMPASS OCR 2026");
    let raster = PreparedRaster {
        width: image.width(),
        height: image.height(),
        image,
    };
    let request = OcrRequest {
        schema: OCR_SCHEMA.to_owned(),
        request_id: "installed-model-clean-english".to_owned(),
        source_kind: OcrSourceKind::RasterImage,
        width: raster.width,
        height: raster.height,
        language_hints: vec!["en".to_owned()],
        image_digest: prepared_raster_digest(&raster),
    };
    let response = engine.recognize(&request, &raster)?;
    let actual = response
        .observations
        .iter()
        .map(|observation| observation.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let expected = normalize("COMPASS OCR 2026");
    let actual = normalize(&actual);
    let distance = edit_distance(&expected, &actual);
    let cer_bps = distance.saturating_mul(10_000) / expected.chars().count().max(1);
    eprintln!("clean-English expected={expected:?} actual={actual:?} cer_bps={cer_bps}");
    assert!(cer_bps <= 500, "clean English CER exceeds 5% release gate");
    Ok(())
}

fn synthetic_clean_english(text: &str) -> RgbImage {
    const SCALE: u32 = 8;
    let mut image = RgbImage::from_pixel(900, 160, Rgb([255, 255, 255]));
    let mut x = 34_u32;
    for character in text.chars() {
        if character == ' ' {
            x += 4 * SCALE;
            continue;
        }
        if let Some(rows) = glyph(character) {
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..5_u32 {
                    if bits & (1 << (4 - column)) != 0 {
                        for dy in 0..SCALE {
                            for dx in 0..SCALE {
                                image.put_pixel(
                                    x + column * SCALE + dx,
                                    48 + u32::try_from(row).unwrap_or(0) * SCALE + dy,
                                    Rgb([0, 0, 0]),
                                );
                            }
                        }
                    }
                }
            }
        }
        x += 6 * SCALE;
    }
    image
}

fn glyph(character: char) -> Option<[u8; 7]> {
    match character {
        'A' => Some([
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ]),
        'C' => Some([
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ]),
        'M' => Some([
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ]),
        'O' => Some([
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ]),
        '0' => Some([
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ]),
        'P' => Some([
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ]),
        'R' => Some([
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ]),
        'S' => Some([
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ]),
        '2' => Some([
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ]),
        '6' => Some([
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ]),
        _ => None,
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (row, left_character) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_character) in right.iter().enumerate() {
            current.push(
                (previous[column + 1] + 1)
                    .min(current[column] + 1)
                    .min(previous[column] + usize::from(left_character != *right_character)),
            );
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}
