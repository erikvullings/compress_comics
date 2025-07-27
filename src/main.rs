use anyhow::{Context, Result};
use clap::Parser;
use crossbeam_channel::{bounded, Receiver, Sender};
use image::ImageReader;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile::TempDir;
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

    let comic_files = if input_path.is_file() {
        vec![detect_comic_file(&input_path)?]
    } else {
        find_comic_files(&input_path)?
    };

    if comic_files.is_empty() {
        println!("No comic files found in the specified path.");
        return Ok(());
    }

    println!("🚀 Found {} comic file(s) to process", comic_files.len());
    println!(
        "Settings: Quality={}, Target Height={}px",
        args.quality, args.target_height
    );
    println!("-----------------------------------------------------");

    let multi_progress = Arc::new(MultiProgress::new());
    let overall_progress = multi_progress.add(ProgressBar::new(comic_files.len() as u64));
    overall_progress.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({eta})")?
            .progress_chars("#>-"),
    );

    let stats = Arc::new(Mutex::new(HashMap::new()));

    comic_files.par_iter().for_each(|comic_file| {
        let file_progress = multi_progress.add(ProgressBar::new(100));
        let style_result = ProgressStyle::default_bar()
            .template("  {msg} [{bar:30.green/yellow}] {percent}%")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ ");
        file_progress.set_style(style_result);
        file_progress.set_message(format!(
            "📖 {}",
            comic_file.path.file_name().unwrap().to_string_lossy()
        ));

        match process_comic_file(comic_file, &args, &file_progress) {
            Ok(file_stats) => {
                let mut stats_map = stats.lock().unwrap();
                stats_map.insert(comic_file.path.clone(), file_stats);
                file_progress.finish_with_message("✅ Complete");
            }
            Err(e) => {
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

fn process_comic_file(
    comic_file: &ComicFile,
    args: &Args,
    progress: &ProgressBar,
) -> Result<ProcessingStats> {
    let original_size = fs::metadata(&comic_file.path)?.len();

    let temp_dir = TempDir::new().context("Failed to create temporary directory")?;
    progress.set_position(10);

    extract_comic(&comic_file, temp_dir.path(), progress)?;
    progress.set_position(30);

    let image_files = find_image_files(temp_dir.path())?;
    let stats = process_images(&image_files, args, progress)?;
    progress.set_position(80);

    let output_path = generate_output_path(&comic_file.path, args.quality);
    create_cbr_archive(temp_dir.path(), &output_path, progress)?;
    progress.set_position(100);

    let compressed_size = fs::metadata(&output_path)?.len();

    Ok(ProcessingStats {
        original_size,
        compressed_size,
        images_processed: stats.0,
        images_skipped: stats.1,
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
            anyhow::bail!("PDF extraction not yet implemented in this Rust version");
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

fn find_image_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut image_files = Vec::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
                match extension.to_lowercase().as_str() {
                    "jpg" | "jpeg" | "png" | "bmp" | "tiff" | "tif" => {
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
            progress_clone.set_position(progress_percent as u64);
        }
    });

    image_files.par_iter().for_each(|image_path| {
        let result = process_single_image(image_path, args);
        sender.send((image_path.clone(), result.is_ok())).unwrap();
    });

    drop(sender);
    thread::sleep(std::time::Duration::from_millis(100));

    let processed = *processed_count.lock().unwrap();
    let skipped = *skipped_count.lock().unwrap();

    Ok((processed, skipped))
}

fn process_single_image(image_path: &Path, args: &Args) -> Result<()> {
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

fn generate_output_path(input_path: &Path, quality: u8) -> PathBuf {
    let parent = input_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = input_path.file_stem().unwrap().to_string_lossy();
    parent.join(format!("{} optimized_webp_q{}.cbr", stem, quality))
}

fn print_summary(stats: &HashMap<PathBuf, ProcessingStats>) {
    println!("\n📊 Processing Summary:");
    println!("-----------------------------------------------------");

    let mut total_original = 0u64;
    let mut total_compressed = 0u64;
    let mut total_images = 0;
    let mut total_skipped = 0;
    let mut files_with_no_savings = 0;

    for (path, stat) in stats {
        let savings = if stat.original_size > stat.compressed_size {
            ((stat.original_size - stat.compressed_size) as f64 / stat.original_size as f64) * 100.0
        } else {
            0.0
        };

        if savings < 5.0 {
            files_with_no_savings += 1;
        }

        println!(
            "📖 {}: {:.1}% savings ({} images processed, {} skipped)",
            path.file_name().unwrap().to_string_lossy(),
            savings,
            stat.images_processed,
            stat.images_skipped
        );

        total_original += stat.original_size;
        total_compressed += stat.compressed_size;
        total_images += stat.images_processed;
        total_skipped += stat.images_skipped;
    }

    let overall_savings = if total_original > total_compressed {
        ((total_original - total_compressed) as f64 / total_original as f64) * 100.0
    } else {
        0.0
    };

    println!("\n🎯 Overall Results:");
    println!("   Total files processed: {}", stats.len());
    println!("   Total images processed: {}", total_images);
    println!("   Total images skipped: {}", total_skipped);
    println!("   Overall size reduction: {:.1}%", overall_savings);
    println!(
        "   Original size: {:.2} MB",
        total_original as f64 / 1_048_576.0
    );
    println!(
        "   Compressed size: {:.2} MB",
        total_compressed as f64 / 1_048_576.0
    );

    if files_with_no_savings > 0 {
        println!(
            "\n💡 {} file(s) were already well-compressed and showed minimal improvement.",
            files_with_no_savings
        );
    }
}
