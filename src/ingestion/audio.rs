use crate::AppResult;
use std::path::Path;
use tokio::process::Command;

pub async fn extract_audio(video_path: &Path, out_path: &Path) -> AppResult<()> {
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video_path.to_str().unwrap_or_default(),
            "-ar",
            "16000",
            "-ac",
            "1",
            "-f",
            "wav",
            out_path.to_str().unwrap_or_default(),
        ])
        .output()
        .await?;

    if !output.status.success() {
        return Err(crate::AppError::external(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

pub async fn create_playback_video(input_path: &Path, out_path: &Path) -> AppResult<()> {
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input_path.to_str().unwrap_or_default(),
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+faststart",
            out_path.to_str().unwrap_or_default(),
        ])
        .output()
        .await?;

    if !output.status.success() {
        return Err(crate::AppError::external(format!(
            "ffmpeg playback transcode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}
