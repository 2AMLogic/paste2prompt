//! Tests for [`export_size_ok`], the size predicate the web app runs before
//! copying an image's pixels out (issue #37), and for its agreement with
//! [`render_export`].

use super::*;
use crate::{Editor, Op, Point, Region};

fn doc(width: u32, height: u32) -> Document {
    Document::new(ImageSize::new(width, height).unwrap())
}

/// An opaque gray image, for the cases that have to reach the renderer.
fn gray(width: u32, height: u32) -> RgbaImage {
    RgbaImage {
        width,
        height,
        pixels: vec![128; width as usize * height as usize * 4],
    }
}

/// The caption band's height for `doc` in an export `width` pixels wide.
fn band_height(width: u32, doc: &Document) -> u32 {
    let mut text = Text::new();
    layout_band(&mut text, &caption_lines(doc), width as f32)
        .height
        .ceil() as u32
}

fn too_large(width: u32, height: u32) -> Result<(), RenderError> {
    Err(RenderError::TooLarge {
        size: (u64::from(width), u64::from(height)),
    })
}

#[test]
fn exactly_40_mp_is_accepted() {
    assert_eq!(MAX_IMAGE_PIXELS, 8_000 * 5_000);
    assert_eq!(export_size_ok(8_000, 5_000, &doc(8_000, 5_000)), Ok(()));
    // Same pixel count, other aspect ratios.
    assert_eq!(export_size_ok(5_000, 8_000, &doc(5_000, 8_000)), Ok(()));
    assert_eq!(export_size_ok(1_250, 32_000, &doc(1_250, 32_000)), Ok(()));
}

#[test]
fn just_over_40_mp_is_rejected_on_the_image_size() {
    // 40 000 001 has no factorization with both sides <= MAX_EXPORT_SIDE;
    // 3015 × 13267 = 40 000 005 is the smallest pixel count over the limit
    // that an image can have.
    assert_eq!(3_015_u64 * 13_267, MAX_IMAGE_PIXELS + 5);
    assert_eq!(
        export_size_ok(3_015, 13_267, &doc(3_015, 13_267)),
        too_large(3_015, 13_267)
    );
    // One column over 8000 × 5000.
    assert_eq!(
        export_size_ok(8_001, 5_000, &doc(8_001, 5_000)),
        too_large(8_001, 5_000)
    );
    // The 42 MP image from the issue.
    assert_eq!(
        export_size_ok(7_000, 6_000, &doc(7_000, 6_000)),
        too_large(7_000, 6_000)
    );
}

#[test]
fn image_sides_are_limited() {
    assert_eq!(
        export_size_ok(MAX_EXPORT_SIDE, 1_000, &doc(MAX_EXPORT_SIDE, 1_000)),
        Ok(())
    );
    assert_eq!(
        export_size_ok(MAX_EXPORT_SIDE + 1, 1_000, &doc(MAX_EXPORT_SIDE + 1, 1_000)),
        too_large(MAX_EXPORT_SIDE + 1, 1_000)
    );
    assert_eq!(
        export_size_ok(1_000, MAX_EXPORT_SIDE + 1, &doc(1_000, MAX_EXPORT_SIDE + 1)),
        too_large(1_000, MAX_EXPORT_SIDE + 1)
    );
}

#[test]
fn the_caption_band_counts_toward_the_side_limit() {
    let band = band_height(1_000, &doc(1_000, 1_000));
    assert!(band > 0);
    let tallest = MAX_EXPORT_SIDE - band;
    assert_eq!(export_size_ok(1_000, tallest, &doc(1_000, tallest)), Ok(()));
    // The image alone fits, but the image plus its caption doesn't.
    assert_eq!(
        export_size_ok(1_000, tallest + 1, &doc(1_000, tallest + 1)),
        too_large(1_000, MAX_EXPORT_SIDE + 1)
    );
}

#[test]
fn narrow_images_are_measured_at_the_minimum_width() {
    // The band of a narrow image is laid out at MIN_EXPORT_WIDTH, where it
    // wraps less (and so is shorter) than it would at the image's own width.
    let band = band_height(MIN_EXPORT_WIDTH, &doc(100, 1_000));
    assert!(band < band_height(100, &doc(100, 1_000)));
    let tallest = MAX_EXPORT_SIDE - band;
    assert_eq!(export_size_ok(100, tallest, &doc(100, tallest)), Ok(()));
    // The reported size is the padded export's, not the image's.
    assert_eq!(
        export_size_ok(100, tallest + 1, &doc(100, tallest + 1)),
        too_large(MIN_EXPORT_WIDTH, MAX_EXPORT_SIDE + 1)
    );
    let plan = plan(ImageSize::new(100, 1_000).unwrap(), &doc(100, 1_000)).unwrap();
    assert_eq!((plan.width, plan.height), (MIN_EXPORT_WIDTH, 1_000 + band));
}

#[test]
fn a_longer_caption_needs_more_room() {
    let width = 1_000;
    let empty = doc(width, 1_000);
    let tallest = MAX_EXPORT_SIDE - band_height(width, &empty);
    let mut ed = Editor::new(ImageSize::new(width, tallest).unwrap());
    let region = Region::rect(Point::new(0.1, 0.1), Point::new(0.2, 0.2)).unwrap();
    ed.add_edit(region, Op::Hint, Some("make this green".into()))
        .unwrap();
    ed.set_instructions("keep the layout");
    let long = ed.into_document();
    assert_eq!(export_size_ok(width, tallest, &doc(width, tallest)), Ok(()));
    assert!(matches!(
        export_size_ok(width, tallest, &long),
        Err(RenderError::TooLarge { .. })
    ));
}

#[test]
fn empty_and_mismatched_sizes_are_rejected() {
    assert_eq!(
        export_size_ok(0, 10, &doc(1, 10)),
        Err(RenderError::EmptyImage)
    );
    assert_eq!(
        export_size_ok(10, 0, &doc(10, 1)),
        Err(RenderError::EmptyImage)
    );
    assert!(matches!(
        export_size_ok(100, 100, &doc(100, 101)),
        Err(RenderError::SizeMismatch { .. })
    ));
}

/// The renderer and the predicate agree, including at the boundaries. Every
/// case that reaches the renderer gets a real pixel buffer, so a rejection
/// can only come from the size checks.
#[test]
fn renderer_agrees_with_the_predicate() {
    let band_720 = band_height(MIN_EXPORT_WIDTH, &doc(100, 100));
    let band_1000 = band_height(1_000, &doc(1_000, 100));
    let cases: [(u32, u32); 6] = [
        (100, 100),
        (1_000, 600),
        // Narrow and tall, at and just past the padded side limit.
        (100, MAX_EXPORT_SIDE - band_720),
        (100, MAX_EXPORT_SIDE - band_720 + 1),
        // Wider, at and just past the side limit with the band.
        (1_000, MAX_EXPORT_SIDE - band_1000),
        (1_000, MAX_EXPORT_SIDE - band_1000 + 1),
    ];
    for (w, h) in cases {
        let d = doc(w, h);
        let predicted = export_size_ok(w, h, &d);
        let rendered = render_pixmap(&gray(w, h), &d).map(|p| (p.width(), p.height()));
        match (&predicted, &rendered) {
            (Ok(()), Ok(size)) => {
                let plan = plan(ImageSize::new(w, h).unwrap(), &d).unwrap();
                assert_eq!(*size, (plan.width, plan.height), "{w}×{h}");
            }
            (Err(p), Err(r)) => assert_eq!(p, r, "{w}×{h}"),
            _ => panic!("{w}×{h}: predicted {predicted:?}, rendered {rendered:?}"),
        }
    }

    // Over the image limits the renderer rejects before reading the pixels,
    // so these need no buffer.
    for (w, h) in [(8_001, 5_000), (3_015, 13_267), (MAX_EXPORT_SIDE + 1, 10)] {
        let d = doc(w, h);
        let image = RgbaImage {
            width: w,
            height: h,
            pixels: Vec::new(),
        };
        assert_eq!(
            render_export(&image, &d).map(|_| ()),
            export_size_ok(w, h, &d),
            "{w}×{h}"
        );
    }
}
