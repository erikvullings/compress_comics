# Comic Compressor

A high-performance Rust application for compressing comic book files (CBR/CBZ/PDF) with parallel processing. Converts images to WebP format for optimal file size reduction while maintaining visual quality.

## Features

- ✅ **Cross-platform compatibility** - Works on Mac, Windows, and Linux
- ✅ **Parallel processing** - Processes multiple files and images simultaneously
- ✅ **Multiple format support** - Handles CBR (RAR), CBZ (ZIP), and PDF files with automatic format detection
- ✅ **Advanced PDF support** - Direct image extraction from PDFs (JPEG, PNG, CMYK, Grayscale)
- ✅ **Automatic folder processing** - Processes all comic files in a directory by default
- ✅ **Glob pattern support** - Process selective files using patterns (e.g., "ABC*.cbr")
- ✅ **Progress visualization** - Docker-style layered progress display
- ✅ **Smart compression** - Skips images that don't benefit from compression
- ✅ **Intelligent file preservation** - Keeps already well-compressed files unchanged (especially RAR archives)
- ✅ **Robust error handling** - Continues processing even with corrupt images
- ✅ **CBR output format** - Always outputs .cbr files regardless of input format
- ✅ **Standalone binary** - No external dependencies required

## Installation

### Download Pre-built Binaries (Recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/erikvullings/compress_comics/releases):

- **Linux (x86_64)**: `compress_comics-x86_64-unknown-linux-gnu.tar.gz`
- **Windows (x86_64)**: `compress_comics-x86_64-pc-windows-msvc.zip`  
- **macOS (Intel)**: `compress_comics-x86_64-apple-darwin.tar.gz`
- **macOS (Apple Silicon)**: `compress_comics-aarch64-apple-darwin.tar.gz`

#### Linux/macOS Installation:
```bash
# Download and extract (replace URL with latest release)
tar -xzf compress_comics-x86_64-unknown-linux-gnu.tar.gz
chmod +x compress_comics
sudo mv compress_comics /usr/local/bin/  # Optional: add to PATH
```

#### Windows Installation:
1. Download the `.zip` file for Windows
2. Extract `compress_comics.exe` 
3. Place in a folder that's in your PATH or run directly

### From Source
```bash
git clone https://github.com/erikvullings/compress_comics.git
cd compress_comics
cargo build --release
```

The compiled binary will be available at `target/release/compress_comics`

### From crates.io
```bash
cargo install compress_comics
```

## Usage

### Process a single file
```bash
compress_comics comic.cbz --quality 85
compress_comics comic.cbr --quality 85
compress_comics comic.pdf --quality 85
```

### Process all comic files in current directory (default behavior)
```bash
compress_comics
```

### Process all comic files in a specific directory
```bash
compress_comics /path/to/comics/
```

### Process files using glob patterns
```bash
# Simple patterns (automatically searches recursively)
compress_comics --glob-pattern "ABC*.cbr"        # Files starting with "ABC" anywhere
compress_comics --glob-pattern "*Killer*.cbr"    # Files containing "Killer" anywhere

# Explicit recursive patterns
compress_comics --glob-pattern "**/De Killer*.cbr"  # Recursive search for "De Killer"
compress_comics --glob-pattern "**/*Volume*/*.cbz"  # Complex nested patterns

# Absolute path patterns
compress_comics --glob-pattern "/full/path/**/Killer*.cbr"  # Full path search

# Current directory only
compress_comics --glob-pattern "*.pdf"              # PDF files in current directory

# Debug your patterns
compress_comics --glob-pattern "pattern" --verbose  # Shows found files before processing
```

### Custom settings
```bash
compress_comics comics/ --quality 75 --target-height 1600
```

### Rename original files (convenient workflow)
```bash
compress_comics comics/ --rename-original --quality 85
# Result: Original files become *_original.ext, compressed files get clean names
```

### Skip already well-compressed files
```bash
compress_comics comics/ --min-savings 10.0  # Only compress if >10% savings possible
# Files with less potential savings are left unchanged (especially useful for RAR archives)
```

## Options

- `--quality` / `-q`: WebP quality (1-100, default: 90)
  - 85-95: High quality, moderate compression
  - 65-80: Balanced quality and size
  - 40-60: Small files, lower quality

- `--target-height` / `-H`: Target height for images in pixels (default: 1800)
- `--max-dimension` / `-m`: Maximum dimension fallback (default: 1200)
- `--rename-original` / `-r`: Rename original file to `<name>_original.<ext>` and give compressed file the original name
- `--glob-pattern` / `-g`: Process only files matching the glob pattern (e.g., "ABC*.cbr", "*.pdf")
- `--min-savings`: Minimum compression savings percentage required to keep compressed file (default: 5.0)
- `--verbose` / `-v`: Enable detailed output with warnings for debugging (disabled by default for clean output)

## Glob Pattern Tips

Glob patterns use wildcards to match file paths:
- `*` matches any characters within a directory name
- `**` matches any number of directories (recursive)
- `?` matches a single character
- `[abc]` matches any character in brackets

### Common Scenarios

**Find files by series name anywhere in directory tree:**
```bash
compress_comics --glob-pattern "**/De Killer*.cbr"
```

**Find files in specific nested structure:**
```bash
compress_comics --glob-pattern "**/Striparchief*/**/De Killer*.cbr"
```

**Find files with specific volume numbers:**
```bash
compress_comics --glob-pattern "**/*Volume 1*.cbr"
compress_comics --glob-pattern "**/*S0[1-3]*.cbr"  # Seasons 1-3
```

**Use verbose mode to debug patterns:**
```bash
compress_comics --glob-pattern "**/Killer*.cbr" --verbose
```
This shows exactly which files were found before processing.

## Output

### Default Behavior
The tool creates new files with the suffix ` optimized_webp_q{quality}.cbr`:
- Input: `MyComic.cbz` → Output: `MyComic optimized_webp_q90.cbr`
- Input: `MyComic.cbr` → Output: `MyComic optimized_webp_q90.cbr`
- Input: `MyComic.pdf` → Output: `MyComic optimized_webp_q90.cbr`

### With `--rename-original` Option
When using `--rename-original`, the compressed file takes the original name:
- `MyComic.cbz` → `MyComic_original.cbz` (backup) + `MyComic.cbr` (compressed)
- `MyComic.cbr` → `MyComic_original.cbr` (backup) + `MyComic.cbr` (compressed)
- `MyComic.pdf` → `MyComic_original.pdf` (backup) + `MyComic.cbr` (compressed)

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

## Real-World Results

### CBR/CBZ Files
```
📖 Amber Blake - 01.cbr: 61.1% savings (77.9 MB saved, 104 images processed, 0 skipped)
📖 Auschwitz - 01.cbr: 67.9% savings (74.9 MB saved, 84 images processed, 0 skipped)

Overall size reduction: 64.3% (152.8 MB saved)
Original size: 237.8 MB → Final size: 85.0 MB
```

### PDF Files
```
📖 Brocéliande - Tome 67.pdf: 76.3% savings (91.1 MB saved, 55 images processed, 0 skipped)

Overall size reduction: 76.3% (91.1 MB saved)
Original size: 119.4 MB → Final size: 28.3 MB
```

### With `--rename-original` Option
```
📖 comic1.cbr: 76.4% savings (84 images processed, 0 skipped)
📖 comic2.pdf: 81.1% savings (55 images processed, 0 skipped)

Before: comic1.cbr (115.75 MB), comic2.pdf (125.21 MB)
After:  comic1_original.cbr (backup), comic2_original.pdf (backup)
        comic1.cbr (27.35 MB), comic2.cbr (23.71 MB)

Total: 229.80 MB → 48.69 MB (78.8% savings)
```

### Why These Results?
- **PDF files often have the highest compression ratios** because they typically contain uncompressed or lightly compressed images
- **CBR/CBZ files vary** depending on original compression - some modern files are already well-optimized
- **WebP format** provides excellent quality-to-size ratio, especially for comic book artwork
- **--rename-original** makes workflow seamless - no manual file management needed

## Technical Details

- **Language**: Rust (standalone binary, no runtime dependencies)
- **Image Processing**: High-quality Lanczos3 resampling
- **Compression**: WebP lossy compression with configurable quality
- **Archive Format**: ZIP-based CBR files (universal comic reader compatibility)
- **Extraction**: 
  - **CBR files**: Native RAR support with ZIP fallback for compatibility
  - **CBZ files**: Native ZIP extraction
  - **PDF files**: Direct embedded image extraction (JPEG, PNG, CMYK, Grayscale)
- **Threading**: Rayon for work-stealing parallelism

## PDF Support Details

The tool provides comprehensive PDF support for comic books:

### ✅ Supported PDF Image Formats
- **JPEG (DCTDecode)**: Direct extraction with no quality loss
- **PNG/Compressed (FlateDecode)**: Decompression and reconstruction
- **Raw RGB/Grayscale**: Uncompressed pixel data extraction
- **CMYK Images**: Automatic conversion to RGB color space

### ⚠️ Unsupported PDF Formats
- **CCITT Fax compression**: Skipped with informative message
- **Complex vector graphics**: Only embedded raster images are extracted
- **Text-only PDFs**: No images to extract

## Limitations

- Output uses ZIP compression for CBR files (not RAR compression, but maintains .cbr extension for compatibility)
- WebP format may not be supported by very old comic readers
- PDF vector graphics are not rasterized (only embedded images are extracted)

## Building for Distribution

To build optimized binaries for distribution:

```bash
cargo build --release
strip target/release/compress_comics  # Optional: reduce binary size
```

The resulting binary is self-contained and can be distributed without any dependencies.
