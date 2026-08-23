//! Managed OAR-OCR inference and deterministic raster normalization.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};

use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage, imageops::FilterType};
use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::oarocr::OAROCRBuilder;
use sha2::{Digest, Sha256};

use crate::models::{ModelProfile, verify_profile};
use crate::{
    OCR_ENGINE_MAX_SIDE, OCR_ENGINE_THREADS, OCR_MAX_OBSERVATIONS_PER_RASTER,
    OCR_MAX_RASTER_LONG_EDGE, OCR_MAX_RASTER_PIXELS, OCR_SCHEMA, OCR_TILE_OVERLAP, OcrError,
    OcrObservation, OcrPoint, OcrRequest, OcrResponse,
};

#[derive(Clone, Debug)]
pub struct PreparedRaster {
    pub image: RgbImage,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct PreparedRasterTile {
    pub ordinal: u32,
    pub x: u32,
    pub y: u32,
    pub raster: PreparedRaster,
}

pub trait OcrEngine {
    fn identity(&self) -> &crate::OcrProfileIdentity;

    fn recognize(
        &self,
        request: &OcrRequest,
        raster: &PreparedRaster,
    ) -> Result<OcrResponse, OcrError>;

    fn recognize_cancellable(
        &self,
        request: &OcrRequest,
        raster: &PreparedRaster,
        cancellation: &AtomicBool,
    ) -> Result<OcrResponse, OcrError> {
        check_cancelled(cancellation)?;
        let response = self.recognize(request, raster)?;
        check_cancelled(cancellation)?;
        Ok(response)
    }
}

pub struct ManagedOarEngine {
    runtime: oar_ocr::oarocr::OAROCR,
    profile: crate::OcrProfileIdentity,
}

impl ManagedOarEngine {
    pub fn load(profile: ModelProfile) -> Result<Self, OcrError> {
        let files = verify_profile(profile)?;
        let session = OrtSessionConfig::default()
            .with_intra_threads(OCR_ENGINE_THREADS)
            .with_inter_threads(OCR_ENGINE_THREADS);
        let runtime = OAROCRBuilder::new(&files.detector, &files.recognizer, &files.dictionary)
            .ort_session(session)
            .image_batch_size(1)
            .region_batch_size(4)
            .build()
            .map_err(|error| OcrError::EngineUnavailable(error.to_string()))?;
        Ok(Self {
            runtime,
            profile: files.identity,
        })
    }
}

impl OcrEngine for ManagedOarEngine {
    fn identity(&self) -> &crate::OcrProfileIdentity {
        &self.profile
    }

    fn recognize(
        &self,
        request: &OcrRequest,
        raster: &PreparedRaster,
    ) -> Result<OcrResponse, OcrError> {
        request.validate()?;
        if raster.width != request.width || raster.height != request.height {
            return Err(OcrError::InvalidRequest(
                "request dimensions do not match prepared raster".to_owned(),
            ));
        }
        if prepared_raster_digest(raster) != request.image_digest {
            return Err(OcrError::InvalidRequest(
                "request digest does not match the prepared raster".to_owned(),
            ));
        }
        let mut results = self
            .runtime
            .predict(vec![raster.image.clone()])
            .map_err(|error| OcrError::Inference(error.to_string()))?;
        if results.len() != 1 {
            return Err(OcrError::InvalidOutput(
                "OCR engine returned an unexpected result count".to_owned(),
            ));
        }
        let result = results
            .pop()
            .ok_or_else(|| OcrError::InvalidOutput("OCR result disappeared".to_owned()))?;
        if result.text_regions.len() > OCR_MAX_OBSERVATIONS_PER_RASTER {
            return Err(OcrError::InvalidOutput(
                "OCR observation count exceeds limit".to_owned(),
            ));
        }
        let mut regions = result
            .text_regions
            .into_iter()
            .filter_map(|region| {
                let text = region.text?.to_string();
                let confidence = region.confidence?;
                Some((region.bounding_box.points, text, confidence))
            })
            .collect::<Vec<_>>();
        regions.sort_by(|left, right| {
            geometry_key(&left.0)
                .cmp(&geometry_key(&right.0))
                .then_with(|| left.1.cmp(&right.1))
        });
        let mut observations = Vec::with_capacity(regions.len());
        for (index, (points, text, confidence)) in regions.into_iter().enumerate() {
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                return Err(OcrError::InvalidOutput(
                    "OCR engine returned invalid confidence".to_owned(),
                ));
            }
            let polygon = points
                .into_iter()
                .map(|point| {
                    Ok(OcrPoint {
                        x: quantize_coordinate(point.x, raster.width)?,
                        y: quantize_coordinate(point.y, raster.height)?,
                    })
                })
                .collect::<Result<Vec<_>, OcrError>>()?;
            observations.push(OcrObservation {
                ordinal: u32::try_from(index)
                    .map_err(|_| OcrError::InvalidOutput("OCR ordinal overflow".to_owned()))?,
                polygon,
                text,
                confidence_bps: (confidence * 10_000.0).round() as u16,
                script: None,
                orientation_degrees: 0,
            });
        }
        let response = OcrResponse {
            schema: OCR_SCHEMA.to_owned(),
            request_id: request.request_id.clone(),
            profile: self.profile.clone(),
            observations,
        };
        response.validate_for(request)?;
        Ok(response)
    }

    fn recognize_cancellable(
        &self,
        request: &OcrRequest,
        raster: &PreparedRaster,
        cancellation: &AtomicBool,
    ) -> Result<OcrResponse, OcrError> {
        check_cancelled(cancellation)?;
        let response = self.recognize(request, raster)?;
        check_cancelled(cancellation)?;
        Ok(response)
    }
}

fn check_cancelled(cancellation: &AtomicBool) -> Result<(), OcrError> {
    if cancellation.load(Ordering::Acquire) {
        Err(OcrError::Cancelled)
    } else {
        Ok(())
    }
}

#[must_use]
pub fn prepared_raster_digest(raster: &PreparedRaster) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raster.width.to_be_bytes());
    hasher.update(raster.height.to_be_bytes());
    hasher.update(raster.image.as_raw());
    format!("{:x}", hasher.finalize())
}

pub fn tile_raster(raster: &PreparedRaster) -> Result<Vec<PreparedRasterTile>, OcrError> {
    crate::validate_dimensions(raster.width, raster.height)?;
    if raster.image.width() != raster.width || raster.image.height() != raster.height {
        return Err(OcrError::InvalidRequest(
            "prepared raster dimensions do not match its pixels".to_owned(),
        ));
    }
    let x_offsets = tile_offsets(raster.width, OCR_ENGINE_MAX_SIDE, OCR_TILE_OVERLAP)?;
    let y_offsets = tile_offsets(raster.height, OCR_ENGINE_MAX_SIDE, OCR_TILE_OVERLAP)?;
    let capacity = x_offsets
        .len()
        .checked_mul(y_offsets.len())
        .ok_or_else(|| OcrError::InvalidRequest("tile count overflow".to_owned()))?;
    let mut tiles = Vec::with_capacity(capacity);
    for y in y_offsets {
        for &x in &x_offsets {
            let width = OCR_ENGINE_MAX_SIDE.min(raster.width.saturating_sub(x));
            let height = OCR_ENGINE_MAX_SIDE.min(raster.height.saturating_sub(y));
            let image = image::imageops::crop_imm(&raster.image, x, y, width, height).to_image();
            let ordinal = u32::try_from(tiles.len())
                .map_err(|_| OcrError::InvalidRequest("tile ordinal overflow".to_owned()))?;
            tiles.push(PreparedRasterTile {
                ordinal,
                x,
                y,
                raster: PreparedRaster {
                    image,
                    width,
                    height,
                },
            });
        }
    }
    Ok(tiles)
}

fn tile_offsets(length: u32, side: u32, overlap: u32) -> Result<Vec<u32>, OcrError> {
    if side == 0 || overlap >= side {
        return Err(OcrError::InvalidRequest(
            "invalid OCR tiling policy".to_owned(),
        ));
    }
    if length <= side {
        return Ok(vec![0]);
    }
    let stride = side - overlap;
    let mut offsets = vec![0_u32];
    while offsets
        .last()
        .is_some_and(|offset| (*offset).saturating_add(side) < length)
    {
        let current = *offsets
            .last()
            .ok_or_else(|| OcrError::InvalidRequest("tile offsets disappeared".to_owned()))?;
        offsets.push(current.saturating_add(stride));
    }
    Ok(offsets)
}

pub fn prepare_raster(bytes: &[u8]) -> Result<PreparedRaster, OcrError> {
    prepare_raster_cancellable(bytes, &AtomicBool::new(false))
}

pub fn prepare_raster_cancellable(
    bytes: &[u8],
    cancellation: &AtomicBool,
) -> Result<PreparedRaster, OcrError> {
    check_cancelled(cancellation)?;
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| OcrError::InvalidRequest(format!("unknown image format: {error}")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(65_535);
    limits.max_image_height = Some(65_535);
    limits.max_alloc = Some(OCR_MAX_RASTER_PIXELS.saturating_mul(8));
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| OcrError::InvalidRequest(format!("image decode failed: {error}")))?;
    let orientation = decoder
        .orientation()
        .map_err(|error| OcrError::InvalidRequest(format!("image metadata failed: {error}")))?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| OcrError::InvalidRequest("image pixel count overflow".to_owned()))?;
    if width == 0 || height == 0 || pixels > OCR_MAX_RASTER_PIXELS {
        return Err(OcrError::InvalidRequest(
            "image dimensions exceed the OCR raster limit".to_owned(),
        ));
    }
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|error| OcrError::InvalidRequest(format!("image decode failed: {error}")))?;
    check_cancelled(cancellation)?;
    decoded.apply_orientation(orientation);
    let width = decoded.width();
    let height = decoded.height();
    let rgba = decoded.to_rgba8();
    let mut rgb = RgbImage::new(width, height);
    for (target, source) in rgb.pixels_mut().zip(rgba.pixels()) {
        let alpha = u16::from(source[3]);
        for channel in 0..3 {
            let foreground = u16::from(source[channel]).saturating_mul(alpha);
            let background = 255_u16.saturating_mul(255_u16.saturating_sub(alpha));
            target[channel] = u8::try_from((foreground + background + 127) / 255)
                .map_err(|_| OcrError::InvalidRequest("alpha composite overflow".to_owned()))?;
        }
    }
    let (rgb, width, height) =
        if width > OCR_MAX_RASTER_LONG_EDGE || height > OCR_MAX_RASTER_LONG_EDGE {
            let scale = f64::from(OCR_MAX_RASTER_LONG_EDGE) / f64::from(width.max(height));
            let resized_width = (f64::from(width) * scale).round().max(1.0) as u32;
            let resized_height = (f64::from(height) * scale).round().max(1.0) as u32;
            (
                image::imageops::resize(&rgb, resized_width, resized_height, FilterType::Triangle),
                resized_width,
                resized_height,
            )
        } else {
            (rgb, width, height)
        };
    crate::validate_dimensions(width, height)?;
    check_cancelled(cancellation)?;
    Ok(PreparedRaster {
        image: rgb,
        width,
        height,
    })
}

fn geometry_key(points: &[oar_ocr::processors::Point]) -> (u32, u32) {
    let min_y = points
        .iter()
        .map(|point| point.y)
        .filter(|value| value.is_finite())
        .fold(f32::INFINITY, f32::min);
    let min_x = points
        .iter()
        .map(|point| point.x)
        .filter(|value| value.is_finite())
        .fold(f32::INFINITY, f32::min);
    (sortable_float(min_y), sortable_float(min_x))
}

fn sortable_float(value: f32) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        value.round() as u32
    }
}

fn quantize_coordinate(value: f32, bound: u32) -> Result<u32, OcrError> {
    if !value.is_finite() || value < 0.0 || value > bound as f32 || bound == 0 {
        return Err(OcrError::InvalidOutput(
            "OCR engine returned out-of-bounds geometry".to_owned(),
        ));
    }
    Ok((value.round() as u32).min(bound - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixels_are_composited_on_white() -> Result<(), Box<dyn std::error::Error>> {
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 0, 0]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image).write_to(&mut encoded, image::ImageFormat::Png)?;
        let prepared = prepare_raster(encoded.get_ref())?;
        assert_eq!(prepared.image.get_pixel(0, 0), &image::Rgb([255, 255, 255]));
        Ok(())
    }

    #[test]
    fn coordinate_quantization_is_bounded() {
        assert_eq!(quantize_coordinate(100.0, 100).ok(), Some(99));
        assert!(quantize_coordinate(f32::NAN, 100).is_err());
        assert!(quantize_coordinate(-1.0, 100).is_err());
    }

    #[test]
    fn cancellation_is_explicit_before_image_decode() {
        let cancellation = AtomicBool::new(true);
        assert!(matches!(
            prepare_raster_cancellable(b"not decoded", &cancellation),
            Err(OcrError::Cancelled)
        ));
    }

    #[test]
    fn exif_orientation_is_applied_once_before_normalization()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = RgbImage::from_pixel(2, 3, image::Rgb([40, 80, 120]));
        let mut jpeg = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(source).write_to(&mut jpeg, image::ImageFormat::Jpeg)?;
        let original = jpeg.into_inner();
        let exif = [
            0x45, 0x78, 0x69, 0x66, 0, 0, 0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1,
            0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut oriented = Vec::with_capacity(original.len() + exif.len() + 4);
        oriented.extend_from_slice(&original[..2]);
        oriented.extend_from_slice(&[0xff, 0xe1, 0, 34]);
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&original[2..]);

        let prepared = prepare_raster(&oriented)?;
        assert_eq!((prepared.width, prepared.height), (3, 2));
        Ok(())
    }

    #[test]
    fn tiles_are_row_major_overlapping_and_cover_the_source() -> Result<(), OcrError> {
        let raster = PreparedRaster {
            image: RgbImage::new(4_000, 3_000),
            width: 4_000,
            height: 3_000,
        };
        let tiles = tile_raster(&raster)?;
        assert_eq!(tiles.len(), 6);
        assert_eq!(
            tiles
                .iter()
                .map(|tile| (
                    tile.ordinal,
                    tile.x,
                    tile.y,
                    tile.raster.width,
                    tile.raster.height
                ))
                .collect::<Vec<_>>(),
            vec![
                (0, 0, 0, 2_048, 2_048),
                (1, 1_920, 0, 2_048, 2_048),
                (2, 3_840, 0, 160, 2_048),
                (3, 0, 1_920, 2_048, 1_080),
                (4, 1_920, 1_920, 2_048, 1_080),
                (5, 3_840, 1_920, 160, 1_080),
            ]
        );
        Ok(())
    }
}
