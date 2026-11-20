// SPDX-License-Identifier: GPL-3.0-or-later
//! The single status item's image: every segment (SF Symbol + text) drawn side by side
//! into one **template** image, so macOS colours it for a light or dark menu bar and
//! for the highlighted (open) state. Dimmed segments use a lower alpha, which template
//! images keep. The image is drawn on demand at the scale of the screen it is on.
//! Always monochrome (black or white, as macOS chooses): no colours, also not for a
//! low battery (the collapsed item then shows that device instead of the glyph).

use crate::ui::{BarView, ItemView};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
#[cfg(debug_assertions)]
use objc2::AllocAnyThread;
use objc2_app_kit::{
    NSColor, NSCompositingOperation, NSFont, NSFontAttributeName, NSFontWeightRegular, NSForegroundColorAttributeName, NSImage,
    NSImageSymbolConfiguration, NSStatusBar, NSStringDrawing,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// Alpha of asleep / disconnected segments (close to AppKit's disabled look).
const DIMMED_ALPHA: f64 = 0.35;
/// Points between a segment's icon and its text, and between segments.
const ICON_TEXT_GAP: f64 = 2.0;
const SEGMENT_GAP: f64 = 9.0;
const SYMBOL_POINT_SIZE: f64 = 14.0;

struct Part {
    icon: Option<(Retained<NSImage>, NSSize)>,
    text: Option<(Retained<NSString>, NSSize)>,
    alpha: f64,
    width: f64,
}

fn symbol(name: &str, description: &str) -> Option<(Retained<NSImage>, NSSize)> {
    let base =
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), Some(&NSString::from_str(description)))?;
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(SYMBOL_POINT_SIZE, unsafe { NSFontWeightRegular });
    let img = base.imageWithSymbolConfiguration(&config).unwrap_or(base);
    let size = img.size();
    Some((img, size))
}

fn text_attributes(font: &NSFont, alpha: f64) -> Retained<NSDictionary<NSString, AnyObject>> {
    let color = NSColor::colorWithWhite_alpha(0.0, alpha);
    let keys: [&NSString; 2] = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font, &color];
    NSDictionary::from_slices(&keys, &values)
}

fn part(v: &ItemView, font: &NSFont) -> Part {
    let alpha = if v.dimmed { DIMMED_ALPHA } else { 1.0 };
    let icon = symbol(v.symbol, &v.tooltip);
    let text = (!v.title.is_empty()).then(|| {
        let s = NSString::from_str(&v.title);
        let size = unsafe { s.sizeWithAttributes(Some(&text_attributes(font, 1.0))) };
        (s, size)
    });
    let mut width = icon.as_ref().map(|(_, s)| s.width).unwrap_or(0.0);
    if let Some((_, s)) = &text {
        if icon.is_some() {
            width += ICON_TEXT_GAP;
        }
        width += s.width.ceil();
    }
    Part { icon, text, alpha, width }
}

/// The app's own glyph (template image from the bundle); SF Symbol fallback.
pub fn neutral_glyph() -> Option<Retained<NSImage>> {
    let img = NSImage::imageNamed(&NSString::from_str("MenuBarIcon"));
    // Development binaries outside the bundle (previews): the resource file itself.
    #[cfg(debug_assertions)]
    let img = img.or_else(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/MenuBarIcon@2x.png");
        let i = NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path))?;
        i.setSize(NSSize::new(18.0, 18.0));
        Some(i)
    });
    let img = img.or_else(|| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("battery.100percent"),
            Some(&NSString::from_str("battbar")),
        )
    })?;
    img.setTemplate(true);
    Some(img)
}

/// Template image for the status item. `None` only if not even the neutral glyph loads.
pub fn image(view: &BarView) -> Option<Retained<NSImage>> {
    if view.segments.is_empty() {
        let img = neutral_glyph()?;
        img.setAccessibilityDescription(Some(&NSString::from_str(&view.tooltip)));
        return Some(img);
    }
    let font = NSFont::menuBarFontOfSize(0.0);
    let parts: Vec<Part> = view.segments.iter().map(|v| part(v, &font)).collect();
    let height = NSStatusBar::systemStatusBar().thickness().max(18.0);
    let width = (parts.iter().map(|p| p.width).sum::<f64>() + SEGMENT_GAP * (parts.len() as f64 - 1.0)).ceil();
    let draw = RcBlock::new(move |_dst: NSRect| -> Bool {
        let mut x = 0.0;
        for p in &parts {
            if let Some((img, s)) = &p.icon {
                let r = NSRect::new(NSPoint::new(x, ((height - s.height) / 2.0).round()), *s);
                img.drawInRect_fromRect_operation_fraction(r, NSRect::ZERO, NSCompositingOperation::SourceOver, p.alpha);
                x += s.width + ICON_TEXT_GAP;
            }
            if let Some((t, s)) = &p.text {
                let y = ((height - s.height) / 2.0).round();
                let attrs = text_attributes(&font, p.alpha);
                unsafe { t.drawAtPoint_withAttributes(NSPoint::new(x, y), Some(&attrs)) };
                x += s.width.ceil();
            }
            if p.text.is_none() && p.icon.is_some() {
                x -= ICON_TEXT_GAP;
            }
            x += SEGMENT_GAP;
        }
        Bool::YES
    });
    let img = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(width, height), false, &draw);
    img.setTemplate(true);
    img.setAccessibilityDescription(Some(&NSString::from_str(&view.tooltip)));
    Some(img)
}

/// Development: write an image as a PNG at `scale` (black + alpha, as macOS gets it).
#[cfg(debug_assertions)]
pub fn write_png(img: &NSImage, path: &std::path::Path, scale: f64) -> Result<(), String> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext};
    let size = img.size();
    let (w, h) = ((size.width * scale).ceil() as isize, (size.height * scale).ceil() as isize);
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            w,
            h,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }
    .ok_or("bitmap")?;
    rep.setSize(size);
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).ok_or("context")?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    img.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new(0.0, 0.0), size),
        NSRect::ZERO,
        NSCompositingOperation::SourceOver,
        1.0,
    );
    NSGraphicsContext::restoreGraphicsState_class();
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }.ok_or("png")?;
    std::fs::write(path, data.to_vec()).map_err(|e| e.to_string())
}
