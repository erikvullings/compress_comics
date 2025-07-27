# Comic Compressor

A high-performance Rust application for compressing comic book files (CBR/CBZ) with parallel processing. Converts images to WebP format for optimal file size reduction while maintaining visual quality.

## Features

- ✅ **Cross-platform compatibility** - Works on Mac, Windows, and Linux
- ✅ **Parallel processing** - Processes multiple files and images simultaneously
- ✅ **Multiple format support** - Handles CBR (RAR) and CBZ (ZIP) files with automatic format detection
- ✅ **Automatic folder processing** - Processes all comic files in a directory by default
- ✅ **Progress visualization** - Docker-style layered progress display
- ✅ **Smart compression** - Skips images that don't benefit from compression
- ✅ **CBR output format** - Always outputs .cbr files regardless of compression method
- ✅ **Standalone binary** - No external dependencies required

## Installation

### From Source
```bash
git clone <repository>
cd compress_comics_rust
cargo build --release
```

The compiled binary will be available at `target/release/compress_comics`

## Usage

### Process a single file
```bash
./compress_comics comic.cbz --quality 85
```

### Process all comic files in current directory (default behavior)
```bash
./compress_comics
```

### Process all comic files in a specific directory
```bash
./compress_comics /path/to/comics/
```

### Custom settings
```bash
./compress_comics comics/ --quality 75 --target-height 1600
```

## Options

- `--quality` / `-q`: WebP quality (1-100, default: 90)
  - 85-95: High quality, moderate compression
  - 65-80: Balanced quality and size
  - 40-60: Small files, lower quality

- `--target-height` / `-H`: Target height for images in pixels (default: 1800)
- `--max-dimension` / `-m`: Maximum dimension fallback (default: 1200)

## Output

The tool creates new files with the suffix ` optimized_webp_q{quality}.cbr`. For example:
- Input: `MyComic.cbz`
- Output: `MyComic optimized_webp_q90.cbr`

## Performance Features

### Parallel Processing
- Files are processed in parallel using all available CPU cores
- Images within each file are also processed in parallel
- Progress is displayed for each file simultaneously

### Smart Compression
- Only compresses images when WebP provides size benefits
- Automatically detects two-page spreads and adjusts processing
- Skips already well-compressed images

### Memory Efficient
- Uses temporary directories for processing
- Automatic cleanup after completion
- Streaming archive processing

## Progress Display

The tool shows progress similar to Docker image downloads:

```
🚀 Found 3 comic file(s) to process
Settings: Quality=90, Target Height=1800px
-----------------------------------------------------
⠋ [00:01:23] [████████████████████████████████████████] 2/3 files (00:00:45)
  📖 Comic1.cbz [████████████████████████████████] 100%
  📖 Comic2.cbz [████████████████░░░░░░░░░░░░░░░░] 65%
  📖 Comic3.cbz [████░░░░░░░░░░░░░░░░░░░░░░░░░░░░] 15%
```

## Summary Report

After processing, the tool provides a detailed summary:

```
📊 Processing Summary:
-----------------------------------------------------
📖 Comic1.cbz: 45.2% savings (23 images processed, 2 skipped)
📖 Comic2.cbz: 38.7% savings (18 images processed, 1 skipped)

🎯 Overall Results:
   Total files processed: 2
   Total images processed: 41
   Total images skipped: 3
   Overall size reduction: 42.1%
   Original size: 125.43 MB
   Compressed size: 72.65 MB

💡 1 file(s) were already well-compressed and showed minimal improvement.
```

## Technical Details

- **Language**: Rust (standalone binary, no runtime dependencies)
- **Image Processing**: High-quality Lanczos3 resampling
- **Compression**: WebP lossy compression with configurable quality
- **Archive Format**: ZIP-based CBR files (universal comic reader compatibility)
- **Extraction**: Native RAR support for CBR files, ZIP fallback for compatibility
- **Threading**: Rayon for work-stealing parallelism

## Limitations

- PDF support is not yet implemented (placeholder exists)
- Output uses ZIP compression for CBR files (not RAR compression, but maintains .cbr extension for compatibility)
- WebP format may not be supported by very old comic readers

## Building for Distribution

To build optimized binaries for distribution:

```bash
cargo build --release
strip target/release/compress_comics  # Optional: reduce binary size
```

The resulting binary is self-contained and can be distributed without any dependencies.
