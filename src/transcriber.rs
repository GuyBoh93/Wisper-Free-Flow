// Whisper transcription via whisper-rs (whisper.cpp bindings).
// Also handles first-run model download from Hugging Face.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn load(model_path: &Path) -> Result<Self> {
        let path_str = model_path
            .to_str()
            .context("model path is not valid UTF-8")?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .context("loading whisper model")?;
        Ok(Self { ctx })
    }

    pub fn transcribe(&self, samples_16k_mono: &[f32], language: &str) -> Result<String> {
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_no_timestamps(true);
        params.set_single_segment(true);
        // Empty or "auto" → don't pin a language; whisper.cpp will detect it
        // (only useful with multilingual models — `.en` models ignore this).
        if !language.is_empty() && language != "auto" {
            params.set_language(Some(language));
        }
        // Smaller attention context = much faster inference for short utterances.
        // Default is 1500; 768 is plenty for typical dictation phrases and gives
        // ~2x speedup. Increase if accuracy on long utterances suffers.
        params.set_audio_ctx(768);
        params.set_n_threads(
            std::thread::available_parallelism()
                .map(|n| n.get() as i32)
                .unwrap_or(4),
        );

        state.full(params, samples_16k_mono)?;

        let mut text = String::new();
        for segment in state.as_iter() {
            text.push_str(&segment.to_string());
        }
        Ok(text.trim().to_string())
    }
}

pub fn ensure_model(model_name: &str, models_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(models_dir)?;
    let filename = format!("ggml-{model_name}.bin");
    let dest = models_dir.join(&filename);
    if dest.exists() {
        tracing::debug!("model cached: {}", dest.display());
        return Ok(dest);
    }
    download_model(&filename, &dest)?;
    Ok(dest)
}

fn download_model(filename: &str, dest: &Path) -> Result<()> {
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}");
    tracing::info!("downloading {url}");

    let mut response = reqwest::blocking::get(&url)
        .with_context(|| format!("GET {url}"))?
        .error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let tmp = dest.with_extension("bin.part");
    let mut file = std::fs::File::create(&tmp)?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_pct = 0u64;

    loop {
        let n = std::io::Read::read(&mut response, &mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        if total > 0 {
            let pct = downloaded * 100 / total;
            if pct >= last_pct + 5 {
                last_pct = pct;
                tracing::info!("download progress: {pct}% ({downloaded}/{total})");
            }
        }
    }
    drop(file);
    std::fs::rename(&tmp, dest).with_context(|| format!("moving {} -> {}", tmp.display(), dest.display()))?;
    tracing::info!("model saved: {}", dest.display());
    Ok(())
}
