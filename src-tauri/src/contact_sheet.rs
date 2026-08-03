//! Tile several frames into one composite image for the vision model.
//!
//! Sending frames as N separate images costs one image slot each, and
//! `MAX_FRAMES_PER_CALL = 10` means a 300-frame job becomes 30 sequential API
//! calls — each one losing the previous call's context down to a 5-segment,
//! 80-character summary. A 3x3 composite carries nine moments in one slot, so
//! the same job is ~4 calls instead of 30, and the model sees temporal
//! adjacency directly rather than having to reassemble it from labels.
//!
//! ## Why gutters instead of burned-in labels
//!
//! Labelling each cell in-image would be ideal, but no font is bundled
//! (`ab_glyph` is a declared-but-unused dependency), and shipping one to draw
//! nine timestamps is not a good trade. Instead each cell is separated by a
//! visible gutter so the grid is unambiguous to segment, and the caller emits an
//! adjacent text part mapping cell number to timestamp in reading order.
//!
//! ## Cell size is a real trade-off
//!
//! Smaller cells fit more moments per call but can make terminal and code text
//! unreadable — and reading on-screen text is exactly what this app asks the
//! model to do. [`DEFAULT_CELL_WIDTH`] and [`DEFAULT_COLUMNS`] are a starting
//! point, not a validated optimum; see the module tests and the PR notes for the
//! A/B that should settle them.

use crate::error::NarratorError;
use crate::models::Frame;
use image::{GenericImage, Rgb, RgbImage};

/// Cell width in pixels. Frames are downscaled to this before tiling.
///
/// The single-frame path uses 1024 px. 512 halves linear resolution, which is
/// the whole saving — and the whole risk.
pub const DEFAULT_CELL_WIDTH: u32 = 512;

/// Cells per row. 3 gives a 3x3 sheet at nine frames per call.
pub const DEFAULT_COLUMNS: u32 = 3;

/// Gutter thickness in pixels between cells.
///
/// Present so the model can tell where one moment ends and the next begins;
/// without it, two visually similar consecutive frames read as one wide image.
const GUTTER: u32 = 4;

/// Gutter colour — mid grey, distinguishable from both dark terminals and light
/// document backgrounds.
const GUTTER_RGB: Rgb<u8> = Rgb([128, 128, 128]);

/// A tiled composite plus the metadata needed to describe it to the model.
#[derive(Debug, Clone)]
pub struct ContactSheet {
    /// Base64 JPEG of the composite.
    pub base64: String,
    /// Source timestamps in reading order (left to right, then top to bottom).
    pub timestamps: Vec<f64>,
    /// Source frame indices in the same order, so `frame_refs` stay meaningful.
    pub frame_indices: Vec<usize>,
    pub columns: u32,
    pub rows: u32,
}

impl ContactSheet {
    /// The text block that tells the model how to read the grid.
    ///
    /// Without this the model has to guess the cell order and the time of each
    /// cell, which is exactly the kind of thing vision models get subtly wrong.
    pub fn describe(&self) -> String {
        let cells: Vec<String> = self
            .timestamps
            .iter()
            .zip(&self.frame_indices)
            .enumerate()
            .map(|(pos, (ts, frame_idx))| {
                format!("cell {} = frame {} at {:.1}s", pos + 1, frame_idx, ts)
            })
            .collect();
        format!(
            "The next image is a {}x{} grid of {} moments from the video, \
             separated by grey lines. Read it left to right, then top to bottom:\n{}",
            self.columns,
            self.rows,
            self.timestamps.len(),
            cells.join("\n")
        )
    }
}

/// Grid dimensions for `count` frames at `columns` per row.
///
/// Separated out because the geometry is the part that silently goes wrong —
/// an off-by-one row means the last frames are cropped away.
pub fn grid_dimensions(count: usize, columns: u32) -> (u32, u32) {
    let columns = columns.max(1);
    if count == 0 {
        return (columns, 0);
    }
    let rows = (count as u32).div_ceil(columns);
    (columns, rows)
}

/// Tile `frames` into one composite.
///
/// Cell height comes from the first readable frame's aspect ratio, so a 16:9
/// source yields 16:9 cells and nothing is letterboxed or stretched. Frames that
/// fail to load are skipped rather than aborting the sheet — one unreadable
/// frame should cost one moment, not the whole call.
///
/// Returns `Ok(None)` when no frame could be read at all.
pub fn build(
    frames: &[Frame],
    columns: u32,
    cell_width: u32,
) -> Result<Option<ContactSheet>, NarratorError> {
    if frames.is_empty() {
        return Ok(None);
    }
    let columns = columns.max(1);
    let cell_width = cell_width.max(16);

    // Load first, so the grid is sized to what actually decoded.
    let mut loaded: Vec<(&Frame, RgbImage)> = Vec::with_capacity(frames.len());
    for frame in frames {
        match image::open(&frame.path) {
            Ok(img) => loaded.push((frame, img.to_rgb8())),
            Err(e) => tracing::warn!(
                "contact sheet: skipping unreadable frame {}: {e}",
                frame.path.display()
            ),
        }
    }
    if loaded.is_empty() {
        return Ok(None);
    }

    // Cell aspect from the first decoded frame.
    let (src_w, src_h) = loaded[0].1.dimensions();
    let cell_height = if src_w == 0 {
        cell_width
    } else {
        ((src_h as f64) * (cell_width as f64) / (src_w as f64)).round() as u32
    }
    .max(16);

    let (columns, rows) = grid_dimensions(loaded.len(), columns);
    // Gutters go *between* cells only, so one fewer than the cell count per axis.
    let sheet_w = columns * cell_width + columns.saturating_sub(1) * GUTTER;
    let sheet_h = rows * cell_height + rows.saturating_sub(1) * GUTTER;

    // Start from the gutter colour so unfilled trailing cells read as empty
    // padding rather than as black frames the model might try to describe.
    let mut sheet = RgbImage::from_pixel(sheet_w, sheet_h, GUTTER_RGB);

    let mut timestamps = Vec::with_capacity(loaded.len());
    let mut frame_indices = Vec::with_capacity(loaded.len());

    for (position, (frame, img)) in loaded.iter().enumerate() {
        let col = (position as u32) % columns;
        let row = (position as u32) / columns;
        let x = col * (cell_width + GUTTER);
        let y = row * (cell_height + GUTTER);

        let resized = image::imageops::resize(
            img,
            cell_width,
            cell_height,
            image::imageops::FilterType::Lanczos3,
        );
        sheet
            .copy_from(&resized, x, y)
            .map_err(|e| NarratorError::FrameExtractionError(format!("tile paste failed: {e}")))?;

        timestamps.push(frame.timestamp_seconds);
        frame_indices.push(frame.index);
    }

    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(sheet)
        .write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Jpeg,
        )
        .map_err(|e| {
            NarratorError::FrameExtractionError(format!("contact sheet encode failed: {e}"))
        })?;

    Ok(Some(ContactSheet {
        base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buf),
        timestamps,
        frame_indices,
        columns,
        rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Write a solid-colour JPEG and return a `Frame` pointing at it.
    fn frame_at(dir: &Path, index: usize, ts: f64, w: u32, h: u32, shade: u8) -> Frame {
        let path = dir.join(format!("f{index}.jpg"));
        RgbImage::from_pixel(w, h, Rgb([shade, shade, shade]))
            .save(&path)
            .unwrap();
        Frame {
            index,
            timestamp_seconds: ts,
            path,
            width: w,
            height: h,
        }
    }

    fn decode(sheet: &ContactSheet) -> RgbImage {
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &sheet.base64)
                .expect("valid base64");
        image::load_from_memory(&bytes)
            .expect("valid JPEG")
            .to_rgb8()
    }

    #[test]
    fn grid_dimensions_round_up_partial_rows() {
        // A dropped row silently loses the last frames.
        assert_eq!(grid_dimensions(9, 3), (3, 3));
        assert_eq!(grid_dimensions(7, 3), (3, 3), "7 frames still needs 3 rows");
        assert_eq!(grid_dimensions(1, 3), (3, 1));
        assert_eq!(grid_dimensions(10, 3), (3, 4));
        assert_eq!(grid_dimensions(0, 3), (3, 0));
    }

    #[test]
    fn grid_dimensions_survive_a_zero_column_request() {
        // Would divide by zero.
        assert_eq!(grid_dimensions(4, 0), (1, 4));
    }

    #[test]
    fn composite_has_the_expected_pixel_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<Frame> = (0..9)
            .map(|i| frame_at(dir.path(), i, i as f64, 320, 180, 10 * i as u8))
            .collect();

        let sheet = build(&frames, 3, 100).unwrap().expect("sheet built");
        assert_eq!((sheet.columns, sheet.rows), (3, 3));

        // Cell height follows the 320x180 source aspect: 100 wide → 56 high.
        let img = decode(&sheet);
        let expected_w = 3 * 100 + 2 * GUTTER;
        let expected_h = 3 * 56 + 2 * GUTTER;
        assert_eq!(img.dimensions(), (expected_w, expected_h));
    }

    #[test]
    fn cells_are_laid_out_in_reading_order() {
        // Distinct shades let us read back which frame landed where. If the
        // layout were column-major the model's cell→time mapping would be wrong
        // for every cell but the first.
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<Frame> = (0..4)
            .map(|i| frame_at(dir.path(), i, i as f64, 100, 100, 40 + 50 * i as u8))
            .collect();

        let sheet = build(&frames, 2, 64).unwrap().expect("sheet");
        let img = decode(&sheet);

        // Sample the centre of each cell and check the shade increases across
        // the top row before moving to the second row.
        let centre = |col: u32, row: u32| -> u8 {
            let x = col * (64 + GUTTER) + 32;
            let y = row * (64 + GUTTER) + 32;
            img.get_pixel(x, y).0[0]
        };
        let (c00, c10, c01, c11) = (centre(0, 0), centre(1, 0), centre(0, 1), centre(1, 1));
        // JPEG is lossy, so compare with tolerance against the source shades.
        for (got, want) in [(c00, 40u8), (c10, 90), (c01, 140), (c11, 190)] {
            assert!(
                (got as i16 - want as i16).abs() < 12,
                "expected ~{want} got {got}"
            );
        }
    }

    #[test]
    fn gutters_separate_the_cells() {
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<Frame> = (0..2)
            .map(|i| frame_at(dir.path(), i, i as f64, 100, 100, 0))
            .collect();
        let sheet = build(&frames, 2, 64).unwrap().expect("sheet");
        let img = decode(&sheet);

        // A pixel inside the gutter between column 0 and 1 must be grey, not
        // black like the frames either side of it.
        let gutter_x = 64 + GUTTER / 2;
        let px = img.get_pixel(gutter_x, 32).0[0];
        assert!(
            px > 90,
            "gutter should be light grey so cell edges are visible, got {px}"
        );
    }

    #[test]
    fn describe_maps_every_cell_to_a_frame_and_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![
            frame_at(dir.path(), 7, 12.5, 100, 100, 10),
            frame_at(dir.path(), 8, 15.0, 100, 100, 20),
        ];
        let sheet = build(&frames, 2, 64).unwrap().expect("sheet");
        let text = sheet.describe();

        assert!(text.contains("2x1 grid"), "{text}");
        assert!(
            text.contains("left to right"),
            "reading order must be stated"
        );
        // Frame indices must be the SOURCE indices, so frame_refs stay valid.
        assert!(text.contains("cell 1 = frame 7 at 12.5s"), "{text}");
        assert!(text.contains("cell 2 = frame 8 at 15.0s"), "{text}");
    }

    #[test]
    fn unreadable_frames_cost_one_cell_not_the_whole_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let mut frames = vec![frame_at(dir.path(), 0, 0.0, 100, 100, 10)];
        frames.push(Frame {
            index: 1,
            timestamp_seconds: 1.0,
            path: PathBuf::from("/nonexistent/gone.jpg"),
            width: 100,
            height: 100,
        });
        frames.push(frame_at(dir.path(), 2, 2.0, 100, 100, 200));

        let sheet = build(&frames, 3, 64).unwrap().expect("sheet still builds");
        assert_eq!(sheet.timestamps.len(), 2, "the missing frame is dropped");
        assert_eq!(sheet.frame_indices, vec![0, 2]);
        // The mapping must not claim a cell for the frame that never loaded.
        assert!(!sheet.describe().contains("frame 1 "));
    }

    #[test]
    fn returns_none_when_nothing_can_be_read() {
        let sheet = build(
            &[Frame {
                index: 0,
                timestamp_seconds: 0.0,
                path: PathBuf::from("/nonexistent/a.jpg"),
                width: 10,
                height: 10,
            }],
            3,
            64,
        )
        .unwrap();
        assert!(sheet.is_none());
        assert!(build(&[], 3, 64).unwrap().is_none());
    }

    #[test]
    fn portrait_sources_produce_portrait_cells() {
        // A phone capture must not be squashed into a landscape cell.
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![frame_at(dir.path(), 0, 0.0, 180, 320, 50)];
        let sheet = build(&frames, 1, 90).unwrap().expect("sheet");
        let img = decode(&sheet);
        assert_eq!(img.dimensions(), (90, 160), "cell must keep 9:16");
    }

    #[test]
    fn a_partial_last_row_still_produces_a_valid_image() {
        // 4 frames in a 3-wide grid leaves two empty cells; the sheet must still
        // decode and the empty area must not read as content.
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<Frame> = (0..4)
            .map(|i| frame_at(dir.path(), i, i as f64, 100, 100, 30))
            .collect();
        let sheet = build(&frames, 3, 64).unwrap().expect("sheet");
        assert_eq!((sheet.columns, sheet.rows), (3, 2));
        let img = decode(&sheet);
        assert_eq!(img.dimensions(), (3 * 64 + 2 * GUTTER, 2 * 64 + GUTTER));
        // Cell 5 (row 1, col 1) is unfilled → gutter grey, not black.
        let px = img.get_pixel(64 + GUTTER + 32, 64 + GUTTER + 32).0[0];
        assert!(px > 90, "unfilled cell should be padding grey, got {px}");
        assert_eq!(sheet.timestamps.len(), 4);
    }

    #[test]
    fn defaults_are_the_documented_starting_point() {
        // Pinned so a change is a deliberate edit rather than a drift.
        assert_eq!(DEFAULT_COLUMNS, 3);
        assert_eq!(DEFAULT_CELL_WIDTH, 512);
    }
}
