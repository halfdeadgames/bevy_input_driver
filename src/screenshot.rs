//! Screenshot requests and saving.

use std::{fmt, fs, io, path::Path};

use bevy::{
    image::Image,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};

use crate::status::DriverStatus;

/// Why a captured screenshot could not be written.
#[derive(Debug)]
pub enum ScreenshotSaveError {
    /// The parent folder could not be created.
    CreateFolder(io::Error),
    /// The captured pixels are in a format that cannot be converted to an image file.
    ConvertImage(String),
    /// Encoding or writing the image file failed.
    WriteFile(String),
}

impl fmt::Display for ScreenshotSaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateFolder(source) => write!(formatter, "cannot create folder: {source}"),
            Self::ConvertImage(source) => write!(formatter, "cannot convert image: {source}"),
            Self::WriteFile(source) => write!(formatter, "cannot write file: {source}"),
        }
    }
}

impl std::error::Error for ScreenshotSaveError {}

/// Queues a primary window capture and records it as pending.
///
/// When the capture arrives, the image is saved to `path` (the format follows the file
/// extension, so use `.png`) and [`DriverStatus`] is updated with the result.
///
/// While [`DriverStatus::window_minimized`] is true, nothing is captured and an error is
/// recorded instead.
pub fn request_screenshot(commands: &mut Commands, status: &mut DriverStatus, path: String) {
    // On Windows, capturing a 0 by 0 window breaks it for good: later captures are 1 by 1 images.
    if status.window_minimized {
        status.record_error(format!(
            "screenshot {path}: window is minimized (0x0), skipped"
        ));
        return;
    }
    status.pending_screenshots.push(path.clone());
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut status: ResMut<DriverStatus>| match save_image(
            captured.image.clone(),
            Path::new(&path),
        ) {
            Ok(()) => status.note_screenshot_saved(&path),
            Err(problem) => status.note_screenshot_failed(&path, &problem.to_string()),
        },
    );
}

fn save_image(image: Image, path: &Path) -> Result<(), ScreenshotSaveError> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        fs::create_dir_all(folder).map_err(ScreenshotSaveError::CreateFolder)?;
    }
    let dynamic_image = image
        .try_into_dynamic()
        .map_err(|source| ScreenshotSaveError::ConvertImage(source.to_string()))?;
    dynamic_image
        .to_rgb8()
        .save(path)
        .map_err(|source| ScreenshotSaveError::WriteFile(source.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screenshots_requested_with(mut status: DriverStatus) -> (DriverStatus, usize) {
        let mut world = World::new();
        request_screenshot(&mut world.commands(), &mut status, "shot.png".to_owned());
        world.flush();
        let capture_count = world.query::<&Screenshot>().iter(&world).count();
        (status, capture_count)
    }

    #[test]
    fn minimized_window_skips_the_capture_and_records_an_error() {
        let (status, capture_count) = screenshots_requested_with(DriverStatus {
            window_minimized: true,
            ..default()
        });
        assert_eq!(capture_count, 0);
        assert!(status.pending_screenshots.is_empty());
        assert_eq!(
            status.errors,
            vec!["screenshot shot.png: window is minimized (0x0), skipped".to_owned()]
        );
    }

    #[test]
    fn visible_window_queues_a_capture() {
        let (status, capture_count) = screenshots_requested_with(DriverStatus::default());
        assert_eq!(capture_count, 1);
        assert_eq!(status.pending_screenshots, vec!["shot.png".to_owned()]);
        assert!(status.errors.is_empty());
    }
}
