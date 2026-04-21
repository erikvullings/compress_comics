# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

compress_comics is a high-performance Rust CLI application for compressing comic book files (CBR/CBZ/PDF) with parallel processing. It converts images to WebP format for optimal file size reduction while maintaining visual quality.

## Common Development Commands

### Building
```bash
cargo build --release     # Production build with optimizations
cargo check               # Fast compile check without building
```

### Testing
```bash
cargo test                # Run tests (no tests currently exist)
```

### Running
```bash
cargo run -- [ARGS]      # Run with arguments
./target/release/compress_comics [ARGS]  # Run release binary directly
```

### Documentation
```bash
cargo doc --open          # Generate and open documentation
```

## Architecture Overview

This is a single-file Rust application (`src/main.rs`) structured as follows:

### Core Components

1. **File Detection & Processing Pipeline**
   - `detect_comic_file()` - Identifies CBR/CBZ/PDF files
   - `find_comic_files()` - Recursively finds comic files in directories
   - `process_comic_file()` - Main processing orchestrator

2. **Archive Extraction**
   - `extract_zip_archive()` - Handles CBZ files and CBR files that are actually ZIP
   - `extract_rar_archive()` - Handles true RAR-based CBR files using unrar crate
   - `extract_pdf_archive()` - Extracts embedded images from PDF files using lopdf

3. **PDF Image Extraction** (Complex subsystem)
   - `extract_images_from_page()` - Traverses PDF page resources to find images
   - `extract_image_from_stream()` - Handles different PDF image formats (JPEG, PNG, raw)
   - `extract_flate_decoded_image()` - Decompresses FlateDecode images
   - `extract_raw_image()` - Processes uncompressed image data
   - Supports RGB, Grayscale, and CMYK color spaces

4. **Image Processing**
   - `process_images()` - Parallel processing coordinator using rayon
   - `process_single_image()` - Individual image resizing and WebP conversion
   - `encode_webp()` - WebP encoding with quality settings

5. **Output Generation**
   - `create_cbr_archive()` - Creates ZIP-based CBR files (universal compatibility)
   - `generate_output_path()` - Handles naming conventions and --rename-original logic

### Key Dependencies

- **rayon** - Work-stealing parallelism for processing multiple files/images
- **image** - Image loading, resizing (Lanczos3), and format conversion
- **webp** - WebP encoding with configurable quality
- **zip** - CBZ extraction and CBR creation
- **unrar** - RAR archive extraction for true CBR files
- **lopdf** - PDF parsing and embedded image extraction
- **glob** - Pattern matching for file selection
- **walkdir** - Recursive directory traversal for finding comic files
- **indicatif** - Progress bars with Docker-style multi-file display
- **clap** - Command-line argument parsing (derive API)
- **tempfile** - Secure temporary directory management
- **anyhow** - Error handling with context chaining
- **crossbeam-channel** - Multi-producer multi-consumer channels for parallel processing

### Processing Flow

1. **Discovery**: Find comic files in input path
2. **Parallel Processing**: Process multiple files simultaneously
3. **Extraction**: Extract images to temporary directory based on format
4. **Image Processing**: Resize and convert to WebP in parallel
5. **Archive Creation**: Package processed images into CBR format
6. **File Management**: Handle renaming logic if --rename-original is used

### Special Features

- **Smart Compression**: Only applies WebP if it reduces file size
- **Intelligent File Preservation**: Skips compression when savings are below threshold (preserves RAR compression benefits)
- **Glob Pattern Support**: Select files using patterns like "ABC*.cbr" via `find_comic_files_by_glob()`
- **Robust Error Handling**: Continues processing even with corrupt images, logging warnings
- **Aspect Ratio Handling**: Detects two-page spreads (aspect ratio > 1.3)
- **Multi-format PDF Support**: Handles JPEG, PNG, CMYK, and raw image data
- **Universal Output**: Always outputs CBR format regardless of input
- **Progress Visualization**: Multi-file progress display similar to Docker

## Performance Optimizations

- Parallel file processing using rayon
- Parallel image processing within each file
- Work-stealing thread pool for load balancing
- Temporary directory cleanup
- Release profile: LTO, single codegen unit, panic=abort, binary stripping (~20-30% size reduction)
- Minimal image crate features (png, jpeg only) to reduce build time and binary size

## Installation

```bash
cargo install compress_comics   # Install from crates.io
```