import os
import tempfile
import subprocess
from functools import lru_cache
from pathlib import Path
from typing import Any

from faster_whisper import WhisperModel
from fastapi import FastAPI, UploadFile, File, Form, HTTPException
from fastapi.responses import JSONResponse

app = FastAPI(title="NexaLearn Whisper-Compatible Service")


@lru_cache(maxsize=1)
def get_model() -> WhisperModel:
    model_size = os.getenv("WHISPER_MODEL_SIZE", "base")
    device = os.getenv("WHISPER_DEVICE", "cpu")
    compute_type = os.getenv("WHISPER_COMPUTE_TYPE", "int8")
    return WhisperModel(model_size, device=device, compute_type=compute_type)


def normalize_audio(input_path: str) -> str:
    source = Path(input_path)
    normalized_path = str(source.with_suffix(".normalized.wav"))
    command = [
        "ffmpeg",
        "-y",
        "-i",
        input_path,
        "-vn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-f",
        "wav",
        normalized_path,
    ]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise HTTPException(status_code=400, detail=f"ffmpeg failed: {result.stderr.strip()}")
    return normalized_path


async def transcribe_upload(file: UploadFile, language: str = "en", task: str = "transcribe") -> dict[str, Any]:
    suffix = Path(file.filename or "input.bin").suffix or ".bin"
    with tempfile.TemporaryDirectory(prefix="nexalearn-whisper-") as temp_dir:
        raw_path = Path(temp_dir) / f"upload{suffix}"
        raw_path.write_bytes(await file.read())

        normalized_path = normalize_audio(str(raw_path))
        model = get_model()
        segments, info = model.transcribe(normalized_path, language=language, task=task, vad_filter=True)

        collected = []
        parts = []
        for segment in segments:
            text = segment.text.strip()
            collected.append({
                "start": float(segment.start),
                "end": float(segment.end),
                "text": text,
            })
            if text:
                parts.append(text)

        return {
            "full_text": " ".join(parts).strip(),
            "segments": collected,
            "language": getattr(info, "language", language),
            "duration_s": getattr(info, "duration", None),
            "task": task,
            "source_filename": file.filename,
        }


@app.get("/healthz")
async def healthz():
    return {"status": "ok"}


@app.post("/transcribe")
async def transcribe(file: UploadFile = File(...), language: str = Form("en"), task: str = Form("transcribe")):
    return JSONResponse(await transcribe_upload(file, language=language, task=task))


@app.post("/v1/audio/transcriptions")
async def openai_transcribe(file: UploadFile = File(...), language: str = Form("en"), task: str = Form("transcribe")):
    payload = await transcribe_upload(file, language=language, task=task)
    return JSONResponse(
        {
            "text": payload["full_text"],
            "language": payload["language"],
            "segments": payload["segments"],
            "duration_s": payload["duration_s"],
        }
    )
