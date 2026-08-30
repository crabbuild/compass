//! Pure-Rust, bounded PDF page rasterization for OCR.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

use compass_ocr::{OCR_MAX_RASTER_LONG_EDGE, OCR_MAX_RASTER_PIXELS, PreparedRaster};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings, render};
use image::RgbImage;

use crate::document::{DocumentError, DocumentLocator};
use crate::limits::{OCR_MAX_AGGREGATE_PIXELS, OCR_MAX_PDF_PAGES};

const PDF_OCR_DPI: f64 = 300.0;
const PDF_POINTS_PER_INCH: f64 = 72.0;
pub const PDF_RASTERIZER_IDENTITY: &str = "hayro/0.7.1@300dpi";

#[derive(Clone, Debug)]
pub struct PdfRasterCandidate {
    pub id: String,
    pub owner: DocumentLocator,
    pub page: u32,
    pub raster: PreparedRaster,
}

pub fn rasterize_pdf_pages(
    bytes: &[u8],
    selected_pages: &[u32],
) -> Result<Vec<PdfRasterCandidate>, DocumentError> {
    rasterize_pdf_pages_cancellable(bytes, selected_pages, &AtomicBool::new(false))
}

pub fn rasterize_pdf_pages_cancellable(
    bytes: &[u8],
    selected_pages: &[u32],
    cancellation: &AtomicBool,
) -> Result<Vec<PdfRasterCandidate>, DocumentError> {
    check_cancelled(cancellation)?;
    if selected_pages.len() > OCR_MAX_PDF_PAGES {
        return Err(DocumentError::Rejected(
            "selected PDF page count exceeds OCR limit".to_owned(),
        ));
    }
    let unique = selected_pages.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != selected_pages.len() || unique.contains(&0) {
        return Err(DocumentError::Rejected(
            "selected PDF pages must be unique and one-based".to_owned(),
        ));
    }
    let owned = bytes.to_vec();
    let pages = selected_pages.to_vec();
    std::panic::catch_unwind(|| rasterize_owned(owned, &pages, cancellation))
        .map_err(|_| DocumentError::Parse("PDF rasterizer panicked".to_owned()))?
}

fn rasterize_owned(
    bytes: Vec<u8>,
    selected_pages: &[u32],
    cancellation: &AtomicBool,
) -> Result<Vec<PdfRasterCandidate>, DocumentError> {
    let pdf = Pdf::new(bytes).map_err(|error| {
        DocumentError::Parse(format!("PDF renderer rejected the document: {error:?}"))
    })?;
    let cache = RenderCache::new();
    let interpreter = InterpreterSettings::default();
    let mut output = Vec::with_capacity(selected_pages.len());
    let mut aggregate_pixels = 0_u64;
    for page_number in selected_pages {
        check_cancelled(cancellation)?;
        let index = usize::try_from(page_number.saturating_sub(1))
            .map_err(|_| DocumentError::Rejected("PDF page index overflow".to_owned()))?;
        let page = pdf.pages().get(index).ok_or_else(|| {
            DocumentError::Parse(format!("PDF page {page_number} does not exist"))
        })?;
        let (point_width, point_height) = page.render_dimensions();
        let (width, height, scale) = bounded_render_dimensions(point_width, point_height)?;
        aggregate_pixels = aggregate_pixels
            .checked_add(u64::from(width) * u64::from(height))
            .ok_or_else(|| DocumentError::Rejected("PDF raster pixel total overflow".to_owned()))?;
        if aggregate_pixels > OCR_MAX_AGGREGATE_PIXELS {
            return Err(DocumentError::Rejected(
                "PDF raster pixels exceed the document OCR limit".to_owned(),
            ));
        }
        let settings = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            width: Some(width),
            height: Some(height),
            bg_color: WHITE,
        };
        let pixmap = render(page, &cache, &interpreter, &settings);
        let raw = pixmap.data_as_u8_slice();
        let expected = usize::from(width)
            .checked_mul(usize::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| DocumentError::Rejected("PDF raster size overflow".to_owned()))?;
        if raw.len() != expected {
            return Err(DocumentError::Parse(
                "PDF renderer returned an invalid raster length".to_owned(),
            ));
        }
        let mut rgb = Vec::with_capacity(expected / 4 * 3);
        for pixel in raw.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        let image = RgbImage::from_raw(u32::from(width), u32::from(height), rgb)
            .ok_or_else(|| DocumentError::Parse("could not construct PDF raster".to_owned()))?;
        output.push(PdfRasterCandidate {
            id: format!("pdf-page-{page_number}"),
            owner: DocumentLocator::Pdf {
                page: *page_number,
                item: 1,
            },
            page: *page_number,
            raster: PreparedRaster {
                image,
                width: u32::from(width),
                height: u32::from(height),
            },
        });
    }
    check_cancelled(cancellation)?;
    Ok(output)
}

fn check_cancelled(cancellation: &AtomicBool) -> Result<(), DocumentError> {
    if cancellation.load(Ordering::Acquire) {
        Err(DocumentError::Ocr(compass_ocr::OcrError::Cancelled))
    } else {
        Ok(())
    }
}

fn bounded_render_dimensions(
    point_width: f32,
    point_height: f32,
) -> Result<(u16, u16, f32), DocumentError> {
    if !point_width.is_finite()
        || !point_height.is_finite()
        || point_width <= 0.0
        || point_height <= 0.0
    {
        return Err(DocumentError::Rejected(
            "PDF page has invalid dimensions".to_owned(),
        ));
    }
    let target_scale = PDF_OCR_DPI / PDF_POINTS_PER_INCH;
    let target_width = f64::from(point_width) * target_scale;
    let target_height = f64::from(point_height) * target_scale;
    let edge_scale = f64::from(OCR_MAX_RASTER_LONG_EDGE) / target_width.max(target_height);
    let pixel_scale = (OCR_MAX_RASTER_PIXELS as f64 / (target_width * target_height)).sqrt();
    let reduction = edge_scale.min(pixel_scale).min(1.0);
    let width = (target_width * reduction).floor().max(1.0);
    let height = (target_height * reduction).floor().max(1.0);
    if width > f64::from(u16::MAX) || height > f64::from(u16::MAX) {
        return Err(DocumentError::Rejected(
            "PDF page raster dimensions exceed renderer limits".to_owned(),
        ));
    }
    let width = width as u16;
    let height = height as u16;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| DocumentError::Rejected("PDF raster pixel overflow".to_owned()))?;
    if pixels > OCR_MAX_RASTER_PIXELS {
        return Err(DocumentError::Rejected(
            "PDF raster exceeds OCR pixel limit".to_owned(),
        ));
    }
    let scale = (f64::from(width) / f64::from(point_width)) as f32;
    Ok((width, height, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_page_pdf() -> Vec<u8> {
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".as_slice(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".as_slice(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources <<>> /Contents 4 0 R >>".as_slice(),
            b"<< /Length 27 >>\nstream\n0 0 0 rg\n12 12 48 48 re f\n\nendstream".as_slice(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        pdf
    }

    #[test]
    fn page_dimensions_are_reduced_deterministically() {
        let (width, height, scale) = bounded_render_dimensions(612.0, 792.0).unwrap_or((0, 0, 0.0));
        assert_eq!((width, height), (2550, 3300));
        assert!(scale > 4.16 && scale < 4.17);

        let (width, height, _) =
            bounded_render_dimensions(20_000.0, 20_000.0).unwrap_or((0, 0, 0.0));
        assert!(u64::from(width) * u64::from(height) <= OCR_MAX_RASTER_PIXELS);
        assert!(u32::from(width) <= OCR_MAX_RASTER_LONG_EDGE);
    }

    #[test]
    fn renders_pdf_pixels_in_process_and_rejects_bad_pages()
    -> Result<(), Box<dyn std::error::Error>> {
        let rendered = rasterize_pdf_pages(&one_page_pdf(), &[1])?;
        assert_eq!(rendered.len(), 1);
        assert_eq!(
            (rendered[0].raster.width, rendered[0].raster.height),
            (300, 300)
        );
        assert!(
            rendered[0]
                .raster
                .image
                .pixels()
                .any(|pixel| pixel.0 != [255, 255, 255])
        );
        assert!(rasterize_pdf_pages(&one_page_pdf(), &[2]).is_err());
        assert!(rasterize_pdf_pages(b"not a PDF", &[1]).is_err());
        Ok(())
    }
}
