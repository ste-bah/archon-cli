# scripts/

Phase-0 CI helper and setup scripts for the archon-cli workspace.

Shell and utility scripts invoked by CI jobs and local developers live here.
No Rust sources; no generated artifacts. Keep each script self-contained and
documented at the top with a short usage comment.

`install-system-deps.sh` is the cross-OS dependency installer. It detects
macOS/Homebrew and common Linux package managers (`apt`, `dnf`, `pacman`,
`zypper`, `apk`) and installs build, PDF/OCR, video ingest, and optional
sandbox packages. Video ingest dependencies include `ffmpeg`, `ffprobe`,
`yt-dlp`, and `whisper-cli` where packaged by the OS.
