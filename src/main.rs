use anyhow::{Context, Result};
use clap::Parser;
use crossbeam_channel::{bounded, Receiver, Sender};
use glob::glob;
use image::ImageReader;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use walkdir::WalkDir;
use zip::{write::FileOptions, ZipWriter};

#[derive(Parser)]
#[command(author, version, about = "Compress comic book files (CBR/CBZ/PDF) with parallel processing", long_about = None)]
struct Args {
    /// Input file or directory to process. If directory, processes all comic files
    #[arg(value_name = "INPUT")]
    input: Option<PathBuf>,

    /// WebP quality (1-100, default: 90)
    #[arg(short, long, default_value = "90")]
    quality: u8,

    /// Target height for images (default: 1800)
    #[arg(short = 'H', long, default_value = "1800")]
    target_height: u32,

    /// Maximum dimension for fallback (default: 1200)
    #[arg(short, long, default_value = "1200")]
    max_dimension: u32,

    /// Rename original file to <name>_original.<ext> and give compressed file the original name
    #[arg(short, long)]
    rename_original: bool,

    /// Glob pattern for file selection (e.g., "ABC*.cbr")
    #[arg(short, long)]
    glob_pattern: Option<String>,

    /// Minimum compression savings required to keep compressed file (default: 5%)
    #[arg(long, default_value = "5.0")]
    min_savings: f64,

    /// Enable verbose output with detailed warnings
    #[arg(short, long)]
    verbose: bool,

    /// Skip image compression - keep original images, just convert format
    #[arg(short = 'S', long)]
    skip_compression: bool,
}

#[derive(Debug)]
struct ComicFile {
    path: PathBuf,
    file_type: ComicType,
}

#[derive(Debug)]
enum ComicType {
    Cbz,
    Cbr,
    Pdf,
}

#[derive(Debug)]
struct ProcessingStats {
    original_size: u64,
    compressed_size: u64,
    images_processed: usize,
    images_skipped: usize,
    compression_skipped: bool,
    output_path: Option<PathBuf>,
    error_message: Option<String>,
    status_message: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.quality < 1 || args.quality > 100 {
        anyhow::bail!("Quality must be between 1 and 100");
    }

    let input_path = args.input.clone().unwrap_or_else(|| PathBuf::from("."));

    if !input_path.exists() {
        anyhow::bail!("Input path does not exist: {}", input_path.display());
    }

    let comic_files = if let Some(pattern) = &args.glob_pattern {
        find_comic_files_by_glob(pattern)?
    } else if input_path.is_file() {
        vec![detect_comic_file(&input_path)?]
    } else {
        find_comic_files(&input_path)?
    };

    if comic_files.is_empty() {
        if args.glob_pattern.is_some() {
            // Error message already printed in find_comic_files_by_glob
        } else {
            println!("No comic files found in the specified path.");
        }
        return Ok(());
    }

    if args.verbose {
        println!("📁 Found files:");
        for file in &comic_files {
            println!("   - {}", file.path.display());
        }
        println!();
    }

    println!("🚀 Found {} comic file(s) to process", comic_files.len());
    if args.skip_compression {
        println!("Mode: Format conversion (no image compression)");
    } else {
        println!(
            "Settings: Quality={}, Target Height={}px",
            args.quality, args.target_height
        );
    }
    println!("-----------------------------------------------------");

    let multi_progress = Arc::new(MultiProgress::new());
    let overall_progress = multi_progress.add(ProgressBar::new(comic_files.len() as u64));
    overall_progress.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} {pos}/{len} files [{elapsed} < {eta}] [{bar:40.cyan/blue}]")?
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );

    let stats = Arc::new(Mutex::new(HashMap::new()));

    comic_files.par_iter().for_each(|comic_file| {
        let file_progress = multi_progress.add(ProgressBar::new(100));
        let style_result = ProgressStyle::default_bar()
            .template("  {msg} [{elapsed_precise}] [{bar:30.green/yellow}] {pos}/{len} images")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ ");
        file_progress.set_style(style_result);
        file_progress.set_message(format!(
            "{}",
            comic_file.path.file_name().unwrap().to_string_lossy()
        ));

        match process_comic_file(comic_file, &args, &file_progress) {
            Ok(file_stats) => {
                let mut stats_map = stats.lock().unwrap();
                
                if let Some(ref status) = file_stats.status_message {
                    file_progress.finish_with_message(format!("{} {} ({} processed, {} skipped)",
                        if status.contains("Format") { "⏭️" } else { "✅" },
                        status, file_stats.images_processed, file_stats.images_skipped));
                } else if file_stats.compression_skipped {
                    file_progress.finish_with_message(format!("⏭️  Skipped - savings below threshold ({} processed, {} skipped)",
                        file_stats.images_processed, file_stats.images_skipped));
                } else {
                    file_progress.finish_with_message(format!("✅ Compressed ({} processed, {} skipped)",
                        file_stats.images_processed, file_stats.images_skipped));
                }
                
                stats_map.insert(comic_file.path.clone(), file_stats);
            }
            Err(e) => {
                // Create error stats entry
                let error_stats = ProcessingStats {
                    original_size: fs::metadata(&comic_file.path).map(|m| m.len()).unwrap_or(0),
                    compressed_size: 0,
                    images_processed: 0,
                    images_skipped: 0,
                    compression_skipped: false,
                    output_path: None,
                    error_message: Some(e.to_string()),
                    status_message: None,
                };
                
                let mut stats_map = stats.lock().unwrap();
                stats_map.insert(comic_file.path.clone(), error_stats);
                
                file_progress.finish_with_message(format!("❌ Failed: {}", e));
            }
        }
        overall_progress.inc(1);
    });

    overall_progress.finish_with_message("🎉 All files processed!");

    print_summary(&stats.lock().unwrap());

    Ok(())
}

fn detect_comic_file(path: &Path) -> Result<ComicFile> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase());

    let file_type = match extension.as_deref() {
        Some("cbz") => ComicType::Cbz,
        Some("cbr") => ComicType::Cbr,
        Some("pdf") => ComicType::Pdf,
        _ => anyhow::bail!("Unsupported file type. Only CBR, CBZ, and PDF files are supported."),
    };

    Ok(ComicFile {
        path: path.to_path_buf(),
        file_type,
    })
}

fn find_comic_files(dir: &Path) -> Result<Vec<ComicFile>> {
    let mut comic_files = Vec::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(comic_file) = detect_comic_file(entry.path()) {
                comic_files.push(comic_file);
            }
        }
    }

    Ok(comic_files)
}

fn find_comic_files_by_glob(pattern: &str) -> Result<Vec<ComicFile>> {
    let mut comic_files = Vec::new();
    
    // Try the pattern as provided first
    let patterns_to_try = vec![
        pattern.to_string(),
        // If pattern doesn't start with / or **, try making it recursive
        if !pattern.starts_with('/') && !pattern.starts_with("**") {
            format!("**/{}", pattern)
        } else {
            pattern.to_string()
        }
    ];
    
    for pattern_attempt in patterns_to_try {
        for entry in glob(&pattern_attempt).context("Failed to read glob pattern")? {
            match entry {
                Ok(path) => {
                    if path.is_file() {
                        if let Ok(comic_file) = detect_comic_file(&path) {
                            comic_files.push(comic_file);
                        }
                    }
                }
                Err(_) => {
                    // Silently skip glob pattern errors
                }
            }
        }
        
        // If we found files with this pattern, don't try others
        if !comic_files.is_empty() {
            break;
        }
    }

    if comic_files.is_empty() {
        println!("⚠️  No comic files found matching pattern: '{}'", pattern);
        println!("💡 Try patterns like:");
        println!("   - \"**/*Killer*.cbr\" (recursive search)");
        println!("   - \"/full/path/**/Killer*.cbr\" (absolute path)");
        println!("   - \"**/De Killer*.cbr\" (your specific case)");
    }

    Ok(comic_files)
}

fn process_comic_file(
    comic_file: &ComicFile,
    args: &Args,
    progress: &ProgressBar,
) -> Result<ProcessingStats> {
    let original_size = fs::metadata(&comic_file.path)?.len();

    let temp_dir = std::env::temp_dir().join("compress_comics_debug");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir)?;
    progress.set_position(10);

    extract_comic(&comic_file, temp_dir.as_path(), progress).with_context(|| "extract_comic failed")?;
    progress.set_position(30);

    let image_files = find_image_files(temp_dir.as_path())?;

    let stats = process_images(&image_files, args, progress).with_context(|| "process_images failed")?;
    progress.set_position(80);

    // Always create compressed file with temporary name first to avoid overwriting original
    let temp_output_path = if args.rename_original {
        let parent = comic_file.path.parent().unwrap_or_else(|| Path::new("."));
        let stem = comic_file.path.file_stem().unwrap().to_string_lossy();
        parent.join(format!("{}_temp_compressed.cbr", stem))
    } else {
        generate_output_path(&comic_file.path, args.quality, false)
    };

    create_cbr_archive(temp_dir.as_path(), &temp_output_path, progress).with_context(|| "create_cbr_archive failed")?;
    progress.set_position(90);

    let compressed_size = fs::metadata(&temp_output_path)?.len();

    // Calculate compression savings
    let savings_percent = if original_size > 0 {
        ((original_size as f64 - compressed_size as f64) / original_size as f64) * 100.0
    } else {
        0.0
    };

    // Check if compression provides significant benefit
    // If no images were processed (all skipped), keep archive as format conversion
    // If --skip-compression, never skip (always create output)
    // If images were processed (WebP converted), always create output regardless of size
    let compression_skipped = if args.skip_compression {
        false
    } else if stats.0 > 0 {
        false
    } else {
        savings_percent < args.min_savings
    };

    if compression_skipped {
        // Remove the compressed file and keep original
        fs::remove_file(&temp_output_path)
            .context("Failed to remove temporary compressed file")?;

        progress.set_position(100);

        return Ok(ProcessingStats {
            original_size,
            compressed_size: original_size, // No compression applied
            images_processed: stats.0,
            images_skipped: stats.1,
            compression_skipped: true,
            output_path: None,
            error_message: None,
            status_message: None,
        });
    }

    // Handle renaming if requested and compression was beneficial
    let final_output_path = if args.rename_original {
        let original_path = &comic_file.path;
        let original_extension = original_path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("cbr");

        let parent = original_path.parent().unwrap_or_else(|| Path::new("."));
        let stem = original_path.file_stem().unwrap().to_string_lossy();
        let backup_path = parent.join(format!("{}_original.{}", stem, original_extension));
        let final_compressed_path = parent.join(format!("{}.cbr", stem));

        // Rename original file to backup name
        fs::rename(original_path, &backup_path)
            .context("Failed to rename original file")?;

        // Rename compressed file to original name
        fs::rename(&temp_output_path, &final_compressed_path)
            .context("Failed to rename compressed file")?;

        final_compressed_path
    } else {
        temp_output_path.clone()
    };

    progress.set_position(100);

    Ok(ProcessingStats {
        original_size,
        compressed_size,
        images_processed: stats.0,
        images_skipped: stats.1,
        compression_skipped: false,
        output_path: Some(final_output_path),
        error_message: None,
        status_message: if stats.0 > 0 {
            None
        } else {
            Some("Format conversion (no recompression)".to_string())
        },
    })
}

fn extract_comic(comic_file: &ComicFile, temp_dir: &Path, _progress: &ProgressBar) -> Result<()> {
    match comic_file.file_type {
        ComicType::Cbz => {
            extract_zip_archive(&comic_file.path, temp_dir)?;
        }
        ComicType::Cbr => {
            // Try RAR first, fallback to ZIP if it fails (some CBR files are actually ZIP)
            if let Err(_) = extract_rar_archive(&comic_file.path, temp_dir) {
                extract_zip_archive(&comic_file.path, temp_dir)
                    .context("Failed to extract CBR file as both RAR and ZIP")?;
            }
        }
        ComicType::Pdf => {
            extract_pdf_archive(&comic_file.path, temp_dir)?;
        }
    }
    Ok(())
}

fn extract_zip_archive(archive_path: &Path, temp_dir: &Path) -> Result<()> {
    let file = File::open(archive_path)?;
    let reader = BufReader::new(file);
    let mut archive = zip::ZipArchive::new(reader)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_path = temp_dir.join(file.name());

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Skip directories - they are created by create_dir_all above
        if file.name().ends_with('/') {
            continue;
        }

        let mut output_file = File::create(&file_path)?;
        std::io::copy(&mut file, &mut output_file)?;
    }

    Ok(())
}

fn extract_rar_archive(archive_path: &Path, temp_dir: &Path) -> Result<()> {
    let archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .map_err(|e| anyhow::anyhow!("Failed to open RAR archive: {:?}", e))?;

    let mut current_archive = archive;

    loop {
        match current_archive.read_header() {
            Ok(Some(archive_with_header)) => {
                // Extract the current file to the temp directory
                let archive_after_extract = archive_with_header
                    .extract_with_base(temp_dir)
                    .map_err(|e| anyhow::anyhow!("Failed to extract RAR entry: {:?}", e))?;

                current_archive = archive_after_extract;
            }
            Ok(None) => {
                // No more files in the archive
                break;
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to read RAR header: {:?}", e));
            }
        }
    }

    Ok(())
}

fn extract_pdf_archive(pdf_path: &Path, temp_dir: &Path) -> Result<()> {
    use lopdf::{Document, Object};

    let doc = Document::load(pdf_path)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    let pages = doc.get_pages();

    // Decode a JP2 or standard image file into an RgbImage
    fn decode_to_rgb(path: &Path) -> Result<image::RgbImage> {
        if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("jp2")).unwrap_or(false) {
            let jp2 = jpeg2k::Image::from_file(path)
                .map_err(|e| anyhow::anyhow!("JP2 decode failed {}: {:?}", path.display(), e))?;
            let px = jp2.get_pixels(None)
                .map_err(|e| anyhow::anyhow!("JP2 get_pixels failed {}: {:?}", path.display(), e))?;
            let (w, h) = (px.width, px.height);
            let rgb = match px.data {
                jpeg2k::ImagePixelData::Rgb8(d) => d,
                jpeg2k::ImagePixelData::Rgba8(d) => d.chunks(4).flat_map(|c| [c[0], c[1], c[2]]).collect(),
                jpeg2k::ImagePixelData::L8(d) => d.iter().flat_map(|&v| [v, v, v]).collect(),
                _ => return Err(anyhow::anyhow!("Unsupported JP2 pixel format in {}", path.display())),
            };
            image::RgbImage::from_raw(w, h, rgb)
                .ok_or_else(|| anyhow::anyhow!("Failed to build RgbImage from {}", path.display()))
        } else {
            Ok(image::ImageReader::open(path)?.decode()?.into_rgb8())
        }
    }

    // Decode a JP2 or standard image as grayscale (used for SMask alpha)
    fn decode_to_luma(path: &Path) -> Result<image::GrayImage> {
        if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("jp2")).unwrap_or(false) {
            let jp2 = jpeg2k::Image::from_file(path)
                .map_err(|e| anyhow::anyhow!("JP2 decode failed {}: {:?}", path.display(), e))?;
            let px = jp2.get_pixels(None)
                .map_err(|e| anyhow::anyhow!("JP2 get_pixels failed {}: {:?}", path.display(), e))?;
            let (w, h) = (px.width, px.height);
            let gray = match px.data {
                jpeg2k::ImagePixelData::L8(d) => d,
                jpeg2k::ImagePixelData::Rgb8(d) => d.chunks(3)
                    .map(|c| (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) as u8)
                    .collect(),
                jpeg2k::ImagePixelData::Rgba8(d) => d.chunks(4)
                    .map(|c| (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) as u8)
                    .collect(),
                _ => return Err(anyhow::anyhow!("Unsupported JP2 pixel format in {}", path.display())),
            };
            image::GrayImage::from_raw(w, h, gray)
                .ok_or_else(|| anyhow::anyhow!("Failed to build GrayImage from {}", path.display()))
        } else {
            Ok(image::ImageReader::open(path)?.decode()?.into_luma8())
        }
    }

    // Porter-Duff OVER: result = overlay * a + base * (1 - a).
    // PDF SMask semantics: alpha=1 → overlay opaque (replaces base),
    // alpha=0 → overlay transparent (base shows). For IA's MRC pages, the
    // JBIG2 mask is binary: 255 where the foreground layer (ink/text)
    // should show, 0 where the background (photo) should show.
    fn composite_over(base: &mut image::RgbImage, overlay: &image::RgbImage, alpha: &image::GrayImage) {
        let (w, h) = (base.width(), base.height());
        for y in 0..h {
            for x in 0..w {
                let a = alpha.get_pixel(x, y).0[0];
                if a == 0 { continue; }
                if a == 255 {
                    base.put_pixel(x, y, *overlay.get_pixel(x, y));
                    continue;
                }
                let af = a as f32 / 255.0;
                let [br, bg, bb] = base.get_pixel(x, y).0;
                let [or, og, ob] = overlay.get_pixel(x, y).0;
                let nr = (or as f32 * af + br as f32 * (1.0 - af)).round() as u8;
                let ng = (og as f32 * af + bg as f32 * (1.0 - af)).round() as u8;
                let nb = (ob as f32 * af + bb as f32 * (1.0 - af)).round() as u8;
                base.put_pixel(x, y, image::Rgb([nr, ng, nb]));
            }
        }
    }

    // Extract a stream to a file; returns empty PathBuf for unsupported filters.
    fn extract_stream(stream: &lopdf::Stream, doc: &Document, temp_dir: &Path, ref_id: &(u32, u16)) -> Result<PathBuf> {
        let base = format!("img_{:04}_{:04}", ref_id.0, ref_id.1);
        let (path, _) = extract_image_from_stream_to(stream, doc, temp_dir, ref_id, &base)?;
        Ok(path)
    }

    // Decode a JBIG2-encoded stream (PDF-embedded, no file header) into a binary GrayImage.
    // PDF embeds JBIG2 using the "embedded" organization defined in Annex D.3.
    // Returns None if decoding fails (caller will skip compositing and use base alone).
    fn decode_jbig2_mask(data: &[u8], width: u32, height: u32) -> Option<image::GrayImage> {
        struct LumaDecoder {
            buf: Vec<u8>,
        }
        impl hayro_jbig2::Decoder for LumaDecoder {
            fn push_pixel(&mut self, black: bool) {
                self.buf.push(if black { 0 } else { 255 });
            }
            fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
                let luma = if black { 0 } else { 255 };
                self.buf.extend(std::iter::repeat(luma).take(chunk_count as usize * 8));
            }
            fn next_line(&mut self) {}
        }

        let img = hayro_jbig2::Image::new_embedded(data, None).ok()?;
        let (pw, ph) = (img.width(), img.height());
        let mut dec = LumaDecoder { buf: Vec::with_capacity((pw * ph) as usize) };
        img.decode(&mut dec).ok()?;
        // The decoder may emit trailing pad bytes past image width; truncate.
        dec.buf.truncate((pw * ph) as usize);
        let gray = image::GrayImage::from_raw(pw, ph, dec.buf)?;
        if pw != width || ph != height {
            Some(image::imageops::resize(&gray, width, height, image::imageops::FilterType::Nearest))
        } else {
            Some(gray)
        }
    }

    for (page_num, (_, page_object_id)) in pages.iter().enumerate() {
        // Collect: (name, image_ref, optional_smask_ref) for non-SMask images, sorted by name
        let mut smask_ref_ids: std::collections::HashSet<(u32, u16)> = std::collections::HashSet::new();
        let mut layers: Vec<(String, (u32, u16), Option<(u32, u16)>)> = Vec::new();

        if let Ok(Object::Dictionary(page_dict)) = doc.get_object(*page_object_id) {
            if let Ok(Object::Dictionary(resources)) = page_dict.get(b"Resources") {
                if let Ok(Object::Dictionary(xobject)) = resources.get(b"XObject") {
                    for (_name, obj_ref) in xobject.iter() {
                        if let Object::Reference(ref_id) = obj_ref {
                            if let Ok(Object::Stream(stream)) = doc.get_object(*ref_id) {
                                if let Ok(Object::Reference(smask_id)) = stream.dict.get(b"SMask") {
                                    smask_ref_ids.insert(*smask_id);
                                }
                            }
                        }
                    }
                    for (name, obj_ref) in xobject.iter() {
                        if let Object::Reference(ref_id) = obj_ref {
                            if smask_ref_ids.contains(ref_id) { continue; }
                            if let Ok(Object::Stream(stream)) = doc.get_object(*ref_id) {
                                if let Ok(Object::Name(subtype)) = stream.dict.get(b"Subtype") {
                                    if subtype == b"Image" {
                                        let smask = stream.dict.get(b"SMask").ok()
                                            .and_then(|o| if let Object::Reference(id) = o { Some(*id) } else { None });
                                        layers.push((String::from_utf8_lossy(name).into_owned(), *ref_id, smask));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if layers.is_empty() { continue; }
        layers.sort_by(|a, b| a.0.cmp(&b.0));

        let output_num = page_num + 1;
        let out_path = temp_dir.join(format!("page_{:04}.png", output_num));

        // Decode all layers and composite bottom-to-top
        let mut composite: Option<image::RgbImage> = None;

        for (_, ref_id, smask_ref) in &layers {
            let layer_rgb = if let Ok(Object::Stream(stream)) = doc.get_object(*ref_id) {
                let path = extract_stream(&stream, &doc, temp_dir, ref_id)?;
                if path == PathBuf::new() { continue; }
                let rgb = decode_to_rgb(&path)?;
                let _ = fs::remove_file(&path);
                rgb
            } else { continue; };

            let (w, h) = (layer_rgb.width(), layer_rgb.height());

            // Get alpha mask for this layer (if it has an SMask)
            let alpha: Option<image::GrayImage> = if let Some(smask_id) = smask_ref {
                if let Ok(Object::Stream(smask_stream)) = doc.get_object(*smask_id) {
                    let filter = smask_stream.dict.get(b"Filter").ok()
                        .and_then(|o| if let Object::Name(n) = o { Some(n.clone()) } else { None });
                    match filter.as_deref() {
                        Some(b"JBIG2Decode") => {
                            decode_jbig2_mask(&smask_stream.content, w, h)
                        }
                        _ => {
                            // Try extracting via the normal path (JPXDecode etc.)
                            let path = extract_stream(&smask_stream, &doc, temp_dir, smask_id)?;
                            if path == PathBuf::new() { None } else {
                                let gray = decode_to_luma(&path).ok();
                                let _ = fs::remove_file(&path);
                                gray
                            }
                        }
                    }
                } else { None }
            } else { None };

            match (&mut composite, alpha) {
                (None, None) => {
                    // Base layer, fully opaque — use directly
                    composite = Some(layer_rgb);
                }
                (None, Some(alpha)) => {
                    // Base layer with mask: composite over white
                    let mut base = image::RgbImage::from_pixel(w, h, image::Rgb([255u8, 255, 255]));
                    composite_over(&mut base, &layer_rgb, &alpha);
                    composite = Some(base);
                }
                (Some(ref mut base), None) => {
                    // Overlay, fully opaque — paint over base entirely
                    *base = layer_rgb;
                }
                (Some(ref mut base), Some(alpha)) => {
                    // Overlay with mask: composite over existing base
                    let (bw, bh) = (base.width(), base.height());
                    let layer_rgb = if layer_rgb.width() != bw || layer_rgb.height() != bh {
                        image::imageops::resize(&layer_rgb, bw, bh, image::imageops::FilterType::Lanczos3)
                    } else { layer_rgb };
                    let alpha = if alpha.width() != bw || alpha.height() != bh {
                        image::imageops::resize(&alpha, bw, bh, image::imageops::FilterType::Nearest)
                    } else { alpha };
                    composite_over(base, &layer_rgb, &alpha);
                }
            }
        }

        if let Some(img) = composite {
            img.save(&out_path).map_err(|e| anyhow::anyhow!("save page {} failed: {:?}", output_num, e))?;
        }
    }

    Ok(())
}

fn extract_image_from_stream_to(
    stream: &lopdf::Stream,
    _doc: &lopdf::Document,
    temp_dir: &Path,
    _ref_id: &(u32, u16),
    base_name: &str,
) -> Result<(PathBuf, usize)> {
    use lopdf::Object;

    // Get image properties
    let width = stream.dict.get(b"Width")
        .ok()
        .and_then(|obj| obj.as_i64().ok())
        .unwrap_or(0);

    let height = stream.dict.get(b"Height")
        .ok()
        .and_then(|obj| obj.as_i64().ok())
        .unwrap_or(0);

    let bits_per_component = stream.dict.get(b"BitsPerComponent")
        .ok()
        .and_then(|obj| obj.as_i64().ok())
        .unwrap_or(8) as u32;

    // Check the filter to determine image format
    if let Ok(Object::Name(filter)) = stream.dict.get(b"Filter") {
        match filter.as_slice() {
            b"DCTDecode" => {
                let output_path = temp_dir.join(format!("{}.jpg", base_name));
                fs::write(&output_path, &stream.content)
                    .map_err(|e| anyhow::anyhow!("Failed to save JPEG image: {:?}", e))?;
                return Ok((output_path, 0));
            }
            b"FlateDecode" => {
                extract_flate_decoded_image(stream, temp_dir, base_name, width as u32, height as u32, bits_per_component)?;
                let output_path = temp_dir.join(format!("{}.png", base_name));
                return Ok((output_path, 0));
            }
            b"CCITTFaxDecode" => {
                return Ok((PathBuf::new(), 0));
            }
            b"JPXDecode" => {
                let output_path = temp_dir.join(format!("{}.jp2", base_name));
                fs::write(&output_path, &stream.content)
                    .map_err(|e| anyhow::anyhow!("Failed to save JPEG 2000 image: {:?}", e))?;
                // Extract ICC profile if present
                extract_icc_profile_to(stream, _doc, temp_dir, base_name)?;
                return Ok((output_path, 0));
            }
            _ => {
                return Ok((PathBuf::new(), 0));
            }
        }
    } else {
        // No filter - raw image data
        extract_raw_image(stream, temp_dir, base_name, width as u32, height as u32, bits_per_component)?;
        let output_path = temp_dir.join(format!("{}.png", base_name));
        return Ok((output_path, 0));
    }
}


fn extract_flate_decoded_image(
    stream: &lopdf::Stream,
    temp_dir: &Path,
    base_name: &str,
    width: u32,
    height: u32,
    bits_per_component: u32,
) -> Result<()> {
    use lopdf::Object;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    let mut decoder = ZlibDecoder::new(stream.content.as_slice());
    let mut decompressed_data = Vec::new();
    decoder.read_to_end(&mut decompressed_data)
        .map_err(|e| anyhow::anyhow!("Failed to decompress image data: {:?}", e))?;

    let color_space = stream.dict.get(b"ColorSpace")
        .ok()
        .and_then(|obj| match obj {
            Object::Name(name) => Some(name.as_slice()),
            _ => None,
        });

    let output_path = temp_dir.join(format!("{}.png", base_name));

    match (color_space.map(|cs| cs), bits_per_component) {
        (Some(b"DeviceRGB"), 8) => {
            let img = image::RgbImage::from_raw(width, height, decompressed_data)
                .ok_or_else(|| anyhow::anyhow!("Failed to create RGB image from raw data"))?;
            image::DynamicImage::ImageRgb8(img).save(&output_path)
                .map_err(|e| anyhow::anyhow!("Failed to save PNG image: {:?}", e))?;
        }
        (Some(b"DeviceGray"), 8) => {
            let img = image::GrayImage::from_raw(width, height, decompressed_data)
                .ok_or_else(|| anyhow::anyhow!("Failed to create grayscale image from raw data"))?;
            image::DynamicImage::ImageLuma8(img).save(&output_path)
                .map_err(|e| anyhow::anyhow!("Failed to save PNG image: {:?}", e))?;
        }
        (Some(b"DeviceCMYK"), 8) => {
            if decompressed_data.len() == (width * height * 4) as usize {
                let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
                for chunk in decompressed_data.chunks(4) {
                    let c = chunk[0] as f32 / 255.0;
                    let m = chunk[1] as f32 / 255.0;
                    let y = chunk[2] as f32 / 255.0;
                    let k = chunk[3] as f32 / 255.0;
                    let r = ((1.0 - c) * (1.0 - k) * 255.0) as u8;
                    let g = ((1.0 - m) * (1.0 - k) * 255.0) as u8;
                    let b = ((1.0 - y) * (1.0 - k) * 255.0) as u8;
                    rgb_data.extend_from_slice(&[r, g, b]);
                }
                let img = image::RgbImage::from_raw(width, height, rgb_data)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create RGB image from CMYK data"))?;
                image::DynamicImage::ImageRgb8(img).save(&output_path)
                    .map_err(|e| anyhow::anyhow!("Failed to save PNG image: {:?}", e))?;
            } else {
                return Err(anyhow::anyhow!("CMYK data size mismatch"));
            }
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

fn extract_raw_image(
    stream: &lopdf::Stream,
    temp_dir: &Path,
    base_name: &str,
    width: u32,
    height: u32,
    bits_per_component: u32,
) -> Result<()> {
    use lopdf::Object;

    let color_space = stream.dict.get(b"ColorSpace")
        .ok()
        .and_then(|obj| match obj {
            Object::Name(name) => Some(name.as_slice()),
            _ => None,
        });

    let output_path = temp_dir.join(format!("{}.png", base_name));

    match (color_space.map(|cs| cs), bits_per_component) {
        (Some(b"DeviceRGB"), 8) => {
            let img = image::RgbImage::from_raw(width, height, stream.content.clone())
                .ok_or_else(|| anyhow::anyhow!("Failed to create RGB image from raw data"))?;
            image::DynamicImage::ImageRgb8(img).save(&output_path)
                .map_err(|e| anyhow::anyhow!("Failed to save PNG image: {:?}", e))?;
        }
        (Some(b"DeviceGray"), 8) => {
            let img = image::GrayImage::from_raw(width, height, stream.content.clone())
                .ok_or_else(|| anyhow::anyhow!("Failed to create grayscale image from raw data"))?;
            image::DynamicImage::ImageLuma8(img).save(&output_path)
                .map_err(|e| anyhow::anyhow!("Failed to save PNG image: {:?}", e))?;
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

fn extract_icc_profile_to(
    stream: &lopdf::Stream,
    doc: &lopdf::Document,
    temp_dir: &Path,
    base_name: &str,
) -> Result<()> {
    use lopdf::Object;

    if let Ok(Object::Array(arr)) = stream.dict.get(b"ColorSpace") {
        for i in 0..arr.len() {
            let is_iccbased = match &arr[i] {
                Object::Name(name) => name.as_slice() == b"ICCBased",
                Object::Reference(ref_id) => {
                    if let Ok(Object::Name(name)) = doc.get_object(*ref_id) {
                        name.as_slice() == b"ICCBased"
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if is_iccbased && i + 1 < arr.len() {
                let profile_ref = match &arr[i + 1] {
                    Object::Reference(ref_id) => *ref_id,
                    _ => continue,
                };

                if let Ok(Object::Stream(profile_stream)) = doc.get_object(profile_ref) {
                    let icc_path = temp_dir.join(format!("{}.icc", base_name));
                    let _ = fs::write(&icc_path, &profile_stream.content);
                    return Ok(());
                }
            }
        }
    }

    // Also check for simple reference
    if let Ok(Object::Reference(colorspace_ref)) = stream.dict.get(b"ColorSpace") {
        if let Ok(Object::Name(name)) = doc.get_object(*colorspace_ref) {
            if name.as_slice() == b"ICCBased" {
                if let Ok(Object::Stream(profile_stream)) = doc.get_object(*colorspace_ref) {
                    let icc_path = temp_dir.join(format!("{}.icc", base_name));
                    let _ = fs::write(&icc_path, &profile_stream.content);
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}


fn find_image_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut image_files = Vec::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
                match extension.to_lowercase().as_str() {
                    "jpg" | "jpeg" | "png" | "bmp" | "tiff" | "tif" | "jp2" => {
                        image_files.push(path.to_path_buf());
                    }
                    _ => {}
                }
            }
        }
    }

    image_files.sort();
    Ok(image_files)
}

fn process_images(
    image_files: &[PathBuf],
    args: &Args,
    progress: &ProgressBar,
) -> Result<(usize, usize)> {
    let (sender, receiver): (Sender<(PathBuf, bool)>, Receiver<(PathBuf, bool)>) = bounded(100);
    let processed_count = Arc::new(Mutex::new(0));
    let skipped_count = Arc::new(Mutex::new(0));
    let total_images = image_files.len();

    let progress_clone = progress.clone();
    let processed_clone = Arc::clone(&processed_count);
    let skipped_clone = Arc::clone(&skipped_count);

    thread::spawn(move || {
        for (_, success) in receiver {
            if success {
                *processed_clone.lock().unwrap() += 1;
            } else {
                *skipped_clone.lock().unwrap() += 1;
            }

            let current = *processed_clone.lock().unwrap() + *skipped_clone.lock().unwrap();
            let progress_percent = 30 + ((current * 50) / total_images);
            // Only update progress every 10% to reduce output noise, plus important milestones
            if progress_percent % 10 == 0 || current == total_images || progress_percent >= 80 {
                progress_clone.set_position(progress_percent as u64);
            }
        }
    });

    image_files.par_iter().for_each(|image_path| {
        let result = process_single_image(image_path, args);
        match &result {
            Err(e) => {
                if args.verbose {
                    eprintln!("Warning: Failed to process image {}: {}. Skipping...", 
                              image_path.display(), e);
                }
                sender.send((image_path.clone(), false)).unwrap();
            }
            Ok(_) => {
                sender.send((image_path.clone(), true)).unwrap();
            }
        }
    });

    drop(sender);
    thread::sleep(std::time::Duration::from_millis(100));

    let processed = *processed_count.lock().unwrap();
    let skipped = *skipped_count.lock().unwrap();

    Ok((processed, skipped))
}

fn process_single_image(image_path: &Path, args: &Args) -> Result<()> {
    // Skip compression: keep image as-is
    if args.skip_compression {
        return Ok(());
    }

    // Handle JPEG 2000 files with ICC profile color management
    if image_path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase() == "jp2")
        .unwrap_or(false)
    {
        let jp2_img = jpeg2k::Image::from_file(image_path)
            .map_err(|e| anyhow::anyhow!("Failed to open JPEG 2000 image: {:?}", e))?;
        let pixels = jp2_img.get_pixels(None)
            .map_err(|e| anyhow::anyhow!("Failed to get JPEG 2000 pixels: {:?}", e))?;

        let (rgb_data, width, height) = match pixels.data {
            jpeg2k::ImagePixelData::Rgb8(data) => (data, pixels.width, pixels.height),
            jpeg2k::ImagePixelData::Rgba8(data) => {
                // Convert RGBA to RGB (strip alpha)
                let mut rgb = Vec::with_capacity(pixels.width as usize * pixels.height as usize * 3);
                for i in (0..data.len()).step_by(4) {
                    if i + 2 < data.len() {
                        rgb.push(data[i]);
                        rgb.push(data[i + 1]);
                        rgb.push(data[i + 2]);
                    }
                }
                (rgb, pixels.width, pixels.height)
            }
            jpeg2k::ImagePixelData::L8(data) => {
                // Grayscale: keep as-is (no color profile needed)
                let img = image::DynamicImage::ImageLuma8(
                    image::GrayImage::from_raw(pixels.width, pixels.height, data.clone())
                        .ok_or_else(|| anyhow::anyhow!("Failed to create grayscale image from JPEG 2000 data"))?
                );
                let webp_path = image_path.with_extension("webp");
                let webp_bytes = encode_webp(&img, args.quality)?;
                if webp_bytes.len() < fs::metadata(image_path)?.len() as usize {
                    fs::write(&webp_path, webp_bytes)?;
                    fs::remove_file(image_path)?;
                }
                return Ok(());
            }
            _ => {
                return Ok(()); // Unsupported format, keep as-is
            }
        };

        // Try to apply ICC profile for color management
        let icc_path = image_path.with_extension("icc");
        let rgb_data = if icc_path.exists() {
            let icc_data = fs::read(&icc_path)
                .map_err(|e| anyhow::anyhow!("Failed to read ICC profile: {:?}", e))?;
            let source_profile = moxcms::ColorProfile::new_from_slice(&icc_data)
                .map_err(|e| anyhow::anyhow!("Failed to load ICC profile: {:?}", e))?;
            let dest_profile = moxcms::ColorProfile::new_srgb();
            let transform = source_profile
                .create_transform_8bit(
                    moxcms::Layout::Rgb,
                    &dest_profile,
                    moxcms::Layout::Rgb,
                    moxcms::TransformOptions::default(),
                )
                .map_err(|e| anyhow::anyhow!("Failed to create color transform: {:?}", e))?;

            let mut transformed = vec![0u8; rgb_data.len()];
            let img_width = width as usize;
            for chunk in rgb_data
                .chunks_exact(img_width * 3)
                .zip(transformed.chunks_exact_mut(img_width * 3))
            {
                transform
                    .transform(chunk.0, chunk.1)
                    .map_err(|e| anyhow::anyhow!("Color transform failed: {:?}", e))?;
            }
            transformed
        } else {
            // No ICC profile: assume sRGB (standard assumption for WebP)
            rgb_data
        };

        // Clean up ICC profile file
        let _ = fs::remove_file(&icc_path);

        let img = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(width, height, rgb_data)
                .ok_or_else(|| anyhow::anyhow!("Failed to create RGB image from transformed JPEG 2000 data"))?,
        );

        let webp_path = image_path.with_extension("webp");
        let webp_bytes = encode_webp(&img, args.quality)?;

        // Always produce WebP for JP2 files (ICC color management takes priority over size)
        fs::write(&webp_path, webp_bytes)?;
        fs::remove_file(image_path)?;
        return Ok(()); // Converted to WebP (counts as processed)
    }

    let img = ImageReader::open(image_path)?.decode()?;

    let (width, height) = (img.width(), img.height());
    let aspect_ratio = width as f32 / height as f32;

    let new_height = args.target_height;
    let new_width = if aspect_ratio > 1.3 {
        (new_height as f32 * aspect_ratio) as u32
    } else {
        (new_height as f32 * aspect_ratio) as u32
    };

    let resized = img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3);

    let webp_path = image_path.with_extension("webp");

    let webp_bytes = encode_webp(&resized, args.quality)?;

    if webp_bytes.len() < fs::metadata(image_path)?.len() as usize {
        fs::write(&webp_path, webp_bytes)?;
        fs::remove_file(image_path)?;
        Ok(())
    } else {
        Err(anyhow::anyhow!("WebP compression didn't reduce file size"))
    }
}

fn encode_webp(img: &image::DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let rgb_img = img.to_rgb8();
    let (width, height) = rgb_img.dimensions();

    let encoder = webp::Encoder::from_rgb(&rgb_img, width, height);
    let encoded = encoder.encode(quality as f32);

    Ok(encoded.to_vec())
}

fn create_cbr_archive(temp_dir: &Path, output_path: &Path, _progress: &ProgressBar) -> Result<()> {
    let file = File::create(output_path)?;
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::<()>::default().compression_method(zip::CompressionMethod::Deflated);

    for entry in WalkDir::new(temp_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            let relative_path = path.strip_prefix(temp_dir)?;

            zip.start_file(relative_path.to_string_lossy(), options)?;
            let file_content = fs::read(path)?;
            zip.write_all(&file_content)?;
        }
    }

    zip.finish()?;
    Ok(())
}

fn generate_output_path(input_path: &Path, quality: u8, rename_original: bool) -> PathBuf {
    let parent = input_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = input_path.file_stem().unwrap().to_string_lossy();
    
    if rename_original {
        // When renaming original, compressed file gets the original name (but as .cbr)
        parent.join(format!("{}.cbr", stem))
    } else {
        // Traditional naming with suffix
        parent.join(format!("{} optimized_webp_q{}.cbr", stem, quality))
    }
}

fn print_summary(stats: &HashMap<PathBuf, ProcessingStats>) {
    println!("\n📊 Processing Summary:");
    println!("=====================================================");

    let mut total_original = 0u64;
    let mut total_compressed = 0u64;
    let mut total_images = 0;
    let mut total_skipped = 0;
    let mut files_compressed = 0u32;
    let mut files_format_converted = 0u32;
    let mut files_status_skipped = 0u32;
    let mut files_with_errors = 0u32;

    for (path, stat) in stats {
        if let Some(error_msg) = &stat.error_message {
            println!("  ❌ {} — {}", path.file_name().unwrap().to_string_lossy(), error_msg);
            files_with_errors += 1;
            continue;
        }

        let name = path.file_name().unwrap().to_string_lossy().to_string();

        if stat.compression_skipped {
            if stat.images_processed == 0 && stat.images_skipped == 0 && stat.original_size > 0 {
                println!("  ⏭️  {} — No images found", name);
            } else if stat.images_processed == 0 && stat.images_skipped > 0 {
                let compressed_mb = stat.compressed_size as f64 / 1_048_576.0;
                println!("  ⏭️  {} — {} images kept as originals ({} MB → {:.1} MB)",
                    name, stat.images_skipped,
                    stat.original_size as f64 / 1_048_576.0, compressed_mb);
            } else {
                let savings_pct = if stat.original_size > 0 {
                    ((stat.original_size as f64 - stat.compressed_size as f64) / stat.original_size as f64) * 100.0
                } else { 0.0 };
                println!("  ⏭️  {} — Savings {:.1}% below threshold ({:.1} MB → {:.1} MB, {} processed, {} skipped)",
                    name, savings_pct,
                    stat.original_size as f64 / 1_048_576.0,
                    stat.compressed_size as f64 / 1_048_576.0,
                    stat.images_processed, stat.images_skipped);
            }
            files_status_skipped += 1;
            total_original += stat.original_size;
            total_compressed += stat.compressed_size;
            total_images += stat.images_processed;
            total_skipped += stat.images_skipped;
        } else if let Some(ref status) = stat.status_message {
            let output_name = stat.output_path
                .as_ref()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            println!("  ⏭️  {} — {} ({:.1} MB → {:.1} MB, {} processed, {} skipped)",
                name, status,
                stat.original_size as f64 / 1_048_576.0,
                stat.compressed_size as f64 / 1_048_576.0,
                stat.images_processed, stat.images_skipped);
            println!("     → {}", output_name);
            files_format_converted += 1;
            total_original += stat.original_size;
            total_compressed += stat.compressed_size;
            total_images += stat.images_processed;
            total_skipped += stat.images_skipped;
        } else {
            let savings_pct = if stat.original_size > stat.compressed_size {
                ((stat.original_size - stat.compressed_size) as f64 / stat.original_size as f64) * 100.0
            } else if stat.original_size == stat.compressed_size {
                0.0
            } else {
                -((stat.compressed_size - stat.original_size) as f64 / stat.original_size as f64) * 100.0
            };
            let output_name = stat.output_path
                .as_ref()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let diff_mb = if stat.original_size >= stat.compressed_size {
                (stat.original_size - stat.compressed_size) as f64 / 1_048_576.0
            } else {
                -((stat.compressed_size - stat.original_size) as f64 / 1_048_576.0)
            };
            println!("  ✅ {} — {:.1}% savings ({:.1} MB {}, {} processed, {} skipped)",
                name, savings_pct, diff_mb.abs(),
                if diff_mb >= 0.0 { "saved" } else { "overhead" },
                stat.images_processed, stat.images_skipped);
            println!("     → {}", output_name);
            files_compressed += 1;
            total_original += stat.original_size;
            total_compressed += stat.compressed_size;
            total_images += stat.images_processed;
            total_skipped += stat.images_skipped;
        }
    }

    let overall_savings = if total_original > total_compressed {
        ((total_original - total_compressed) as f64 / total_original as f64) * 100.0
    } else {
        0.0
    };

    println!("\n  ── Files ──");
    println!("    Successfully compressed:       {}", files_compressed);
    if files_format_converted > 0 {
        println!("    Format converted:              {}", files_format_converted);
    }
    if files_status_skipped > 0 {
        println!("    Skipped (no improvement):      {}", files_status_skipped);
    }
    if files_with_errors > 0 {
        println!("    With errors:                   {}", files_with_errors);
    }

    println!("\n  ── Images ──");
    println!("    Processed:  {}", total_images);
    println!("    Skipped:    {}", total_skipped);

    println!("\n  ── Size ──");
    let total_savings_mb = (total_original - total_compressed) as f64 / 1_048_576.0;
    println!("    Original:    {:.2} MB", total_original as f64 / 1_048_576.0);
    println!("    Compressed:  {:.2} MB", total_compressed as f64 / 1_048_576.0);
    if total_original > total_compressed {
        println!("    Saved:       {:.2} MB ({:.1}% reduction)", total_savings_mb, overall_savings);
    } else {
        println!("    No reduction achieved");
    }

    if files_status_skipped > 0 {
        println!("\n  💡 {} file(s) skipped — compression offered no benefit.", files_status_skipped);
    }

    if files_with_errors > 0 {
        println!("\n  ⚠️  {} file(s) had errors.", files_with_errors);
    }
}
