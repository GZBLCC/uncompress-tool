use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use zip::ZipArchive;
use flate2::read::GzDecoder;
use tar::Archive;
use std::fs::File;
use std::io::Read;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Unzip Tool",
        options,
        Box::new(|cc| {
            // Configure fonts
            let mut fonts = egui::FontDefinitions::default();
            
            // Add Noto Sans CJK as the primary font
            // Try to load custom font
            match std::env::current_dir() {
                Ok(current_dir) => {
                    let font_path = current_dir.join("fonts").join("NotoSansCJK-Regular.ttc");
                    println!("Looking for font at: {}", font_path.display());
                    match std::fs::read(&font_path) {
                        Ok(font_data) => {
                            fonts.font_data.insert(
                                "noto_sans_cjk".to_owned(),
                                egui::FontData::from_owned(font_data)
                            );
                            
                            // Set as default font
                            fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap()
                                .insert(0, "noto_sans_cjk".to_owned());
                            
                            // Configure text styles to use the new font
                            fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap()
                                .insert(0, "noto_sans_cjk".to_owned());
                        },
                        Err(_) => {
                            eprintln!("Failed to load font file");
                        }
                    }
                },
                Err(_) => {
                    eprintln!("Failed to get current directory");
                }
            }
            
            // Set as default font
            fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap()
                .insert(0, "noto_sans_cjk".to_owned());
            
            // Configure text styles to use the new font
            fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap()
                .insert(0, "noto_sans_cjk".to_owned());
            
            cc.egui_ctx.set_fonts(fonts);
            
            Box::new(UnzipApp::default())
        }),
    )
}

struct UnzipApp {
    input_file: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    status: String,
    status_color: egui::Color32,
    is_extracting: bool,
    extraction_progress: f32,
    progress_receiver: Option<mpsc::Receiver<(f32, bool)>>,
    status_receiver: Option<mpsc::Receiver<(String, egui::Color32)>>,
    file_list: Vec<(String, String, String)>, // (filename, file_type, content_or_info)
    dark_mode: bool,
}

impl Default for UnzipApp {
    fn default() -> Self {
        Self {
            input_file: None,
            output_dir: None,
            status: "Ready".to_string(),
            status_color: egui::Color32::WHITE,
            is_extracting: false,
            extraction_progress: 0.0,
            progress_receiver: None,
            status_receiver: None,
            file_list: Vec::new(),
            dark_mode: false,
        }
    }
}

impl eframe::App for UnzipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for progress updates
        if let Some(receiver) = self.progress_receiver.take() {
            while let Ok((progress, is_finished)) = receiver.try_recv() {
                self.extraction_progress = progress;
                if is_finished {
                    self.is_extracting = false;
                } else {
                    // Put the receiver back if not finished
                    self.progress_receiver = Some(receiver);
                    break;
                }
            }
        }
        
        // Check for status updates
        if let Some(receiver) = self.status_receiver.take() {
            while let Ok((status, color)) = receiver.try_recv() {
                self.status = status;
                self.status_color = color;
            }
            self.status_receiver = None;
        }
        
        // Set visual style based on dark mode
        let mut style = (*ctx.style()).clone();
        if self.dark_mode {
            style.visuals = egui::Visuals::dark();
        } else {
            style.visuals = egui::Visuals::light();
        }
        ctx.set_style(style);

        egui::CentralPanel::default().show(ctx, |ui| {
            // Theme toggle button
            ui.horizontal(|ui| {
                ui.label("主题:");
                if ui.button(if self.dark_mode { "🌙 暗色" } else { "☀️ 亮色" }).clicked() {
                    self.dark_mode = !self.dark_mode;
                }
            });
            ui.separator();
            ui.heading("解压工具");

            ui.horizontal(|ui| {
                if ui.button("选择压缩文件").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.input_file = Some(path.clone());
                        self.file_list = list_archive_contents(&path)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(f, c)| (f.clone(), get_file_type(&f), c))
                            .collect();
                    }
                }
                ui.label(format!("已选择: {}", 
                    self.input_file.as_ref()
                        .and_then(|p| p.to_str())
                        .unwrap_or("无")));
            });

            ui.horizontal(|ui| {
                if ui.button("选择输出目录").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_dir = Some(path);
                    }
                }
                ui.label(format!("已选择: {}", 
                    self.output_dir.as_ref()
                        .and_then(|p| p.to_str())
                        .unwrap_or("无")));
            });

            ui.separator();

            if ui.button("解压").clicked() && !self.is_extracting {
                if let (Some(input), Some(output)) = (&self.input_file, &self.output_dir) {
                    self.is_extracting = true;
                    self.status = "正在解压...".to_string();
                    
                    let input = input.clone();
                    let output = output.clone();
                    let ctx = ctx.clone();
                    
                    let (progress_tx, progress_rx) = mpsc::channel();
                    let (status_tx, status_rx) = mpsc::channel();
                    self.progress_receiver = Some(progress_rx);
                    
                    thread::spawn(move || {
                        let result = match input.extension().and_then(|ext| ext.to_str()) {
                            Some("zip") => extract_zip(&input, &output, progress_tx.clone()),
                            Some("gz") => extract_tar_gz(&input, &output, progress_tx.clone()),
                            _ => Err("不支持的文件格式".into()),
                        };

                        ctx.request_repaint();
                        match result {
                            Ok(_) => {
                                let _ = status_tx.send((
                                    "解压成功！".to_string(), 
                                    egui::Color32::GREEN
                                ));
                            },
                            Err(e) => {
                                let error_msg = match e.to_string().as_str() {
                                    "Output directory is read-only" => "输出目录是只读的，请检查权限",
                                    "Unsupported file format" => "不支持的文件格式",
                                    "Failed to read archive" => "无法读取压缩文件，文件可能已损坏",
                                    "Failed to create output directory" => "无法创建输出目录，请检查路径和权限",
                                    "Failed to write file" => "无法写入文件，磁盘可能已满或没有权限",
                                    _ => "解压过程中发生未知错误",
                                };
                                
                                let _ = status_tx.send((
                                    format!("错误: {}", error_msg),
                                    egui::Color32::RED
                                ));
                            }
                        }
                    });
                } else {
                    self.status = "请同时选择压缩文件和输出目录".to_string();
                    self.status_color = egui::Color32::RED;
                }
            }

            // Show retry button if last operation failed
            if self.status_color == egui::Color32::RED {
                ui.horizontal(|ui| {
                    if ui.button("重试").clicked() {
                        self.status = "准备重试...".to_string();
                        self.status_color = egui::Color32::WHITE;
                    }
                });
            }

            ui.separator();
            
            // Show file list organized by type
            ui.heading("文件内容");
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Group files by type
                let mut grouped_files = std::collections::HashMap::new();
                for (file, file_type, content) in &self.file_list {
                    grouped_files.entry(file_type.clone())
                        .or_insert_with(Vec::new)
                        .push((file.clone(), content.clone()));
                }

                // Show each file type in a collapsible section
                for (file_type, files) in grouped_files {
                    let icon = get_file_icon(&file_type).unwrap_or("📄");
                    let header = format!("{} {} ({})", icon, file_type, files.len());
                    
                    egui::CollapsingHeader::new(header)
                        .default_open(true)
                        .show(ui, |ui| {
                            for (file, content) in files {
                                ui.horizontal(|ui| {
                                    ui.label(icon);
                                    ui.label(file);
                                });
                                
                                if file_type == "text" {
                                    ui.separator();
                                    egui::ScrollArea::vertical().show(ui, |ui| {
                                        ui.label(content);
                                    });
                                    ui.separator();
                                } else {
                                    ui.label(content);
                                }
                            }
                        });
                }
            });
            
            ui.separator();
            
            // Show progress bar when extracting
            if self.is_extracting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add(egui::ProgressBar::new(self.extraction_progress)
                        .show_percentage()
                        .animate(true)
                        .text(format!("解压中... {:.1}%", self.extraction_progress * 100.0)));
                });
            }
            
            // Show status with color and icon
            ui.horizontal(|ui| {
                if self.status_color == egui::Color32::RED {
                    ui.label("❌");
                } else if self.status_color == egui::Color32::GREEN {
                    ui.label("✅");
                } else {
                    ui.label("ℹ️");
                }
                ui.colored_label(self.status_color, &self.status);
            });
        });
    }
}

fn extract_zip(zip_path: &PathBuf, output_dir: &PathBuf, progress_sender: mpsc::Sender<(f32, bool)>) -> Result<(), Box<dyn std::error::Error>> {
    // Check write permissions for output directory
    let metadata = std::fs::metadata(output_dir)?;
    let permissions = metadata.permissions();
    if permissions.readonly() {
        return Err("Output directory is read-only".into());
    }

    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let total_files = archive.len() as f32;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = output_dir.join(file.mangled_name());

        if file.name().ends_with('/') {
            // Create directory with appropriate permissions
            std::fs::create_dir_all(&outpath)?;
            let mut dir_perms = std::fs::metadata(&outpath)?.permissions();
            dir_perms.set_readonly(false);
            std::fs::set_permissions(&outpath, dir_perms)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                    let mut parent_perms = std::fs::metadata(p)?.permissions();
                    parent_perms.set_readonly(false);
                    std::fs::set_permissions(p, parent_perms)?;
                }
            }
            
            // Create file with appropriate permissions
            let mut outfile = File::create(&outpath)?;
            let mut file_perms = std::fs::metadata(&outpath)?.permissions();
            file_perms.set_readonly(false);
            std::fs::set_permissions(&outpath, file_perms)?;
            
            std::io::copy(&mut file, &mut outfile)?;
        }

        // Send progress update
        let progress = (i as f32 + 1.0) / total_files;
        let _ = progress_sender.send((progress, false));
    }

    // Send final completion status
    let _ = progress_sender.send((1.0, true));
    Ok(())
}

fn get_file_type(filename: &str) -> String {
    if filename.ends_with('/') {
        return "folder".to_string();
    }
    
    match filename.split('.').last() {
        Some("txt") => "text".to_string(),
        Some("jpg") | Some("jpeg") | Some("png") | Some("gif") => "image".to_string(),
        Some("pdf") => "pdf".to_string(),
        Some("zip") | Some("gz") | Some("tar") => "archive".to_string(),
        Some("mp3") | Some("wav") => "audio".to_string(),
        Some("mp4") | Some("avi") => "video".to_string(),
        Some("exe") => "executable".to_string(),
        _ => "file".to_string(),
    }
}

fn get_file_icon(file_type: &str) -> Option<&'static str> {
    match file_type {
        "folder" => Some("📁"),
        "text" => Some("📝"),
        "image" => Some("🖼️"),
        "pdf" => Some("📄"),
        "archive" => Some("📦"),
        "audio" => Some("🎵"),
        "video" => Some("🎥"),
        "executable" => Some("⚙️"),
        _ => None,
    }
}

fn list_archive_contents(archive_path: &PathBuf) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    
    match archive_path.extension().and_then(|ext| ext.to_str()) {
        Some("zip") => {
            let file = File::open(archive_path)?;
            let mut archive = ZipArchive::new(file)?;
            
            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let name = file.name().to_string();
                let content = if name.ends_with(".txt") {
                    let mut content = Vec::new();
                    file.read_to_end(&mut content)?;
                    String::from_utf8_lossy(&content).to_string()
                } else {
                    format!("Type: {} | Size: {} bytes", get_file_type(&name), file.size())
                };
                files.push((name, content));
            }
        },
        Some("gz") => {
            let tar_gz = File::open(archive_path)?;
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);
            
            for entry in archive.entries()? {
                let mut entry = entry?;
                let name = entry.path()?.to_string_lossy().to_string();
                let content = if name.ends_with(".txt") {
                    let mut content = Vec::new();
                    entry.read_to_end(&mut content)?;
                    String::from_utf8_lossy(&content).to_string()
                } else {
                    format!("Type: {} | Size: {} bytes", get_file_type(&name), entry.size())
                };
                files.push((name, content));
            }
        },
        _ => return Err("Unsupported file format".into()),
    }
    
    Ok(files)
}

fn extract_tar_gz(tar_gz_path: &PathBuf, output_dir: &PathBuf, progress_sender: mpsc::Sender<(f32, bool)>) -> Result<(), Box<dyn std::error::Error>> {
    // Check write permissions for output directory
    let metadata = std::fs::metadata(output_dir)?;
    let permissions = metadata.permissions();
    if permissions.readonly() {
        return Err("Output directory is read-only".into());
    }

    let tar_gz = File::open(tar_gz_path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);

    // Get total number of entries for progress calculation
    let entries: Vec<_> = archive.entries()?.collect();
    let total_entries = entries.len() as f32;

    // Set permissions for extracted files and directories
    for (i, entry) in entries.into_iter().enumerate() {
        let mut entry = entry?;
        let path = output_dir.join(entry.path()?);
        
        // Calculate and send progress
        let progress = (i as f32 + 1.0) / total_entries;
        let _ = progress_sender.send((progress, false));
        
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&path)?;
            let mut dir_perms = std::fs::metadata(&path)?.permissions();
            dir_perms.set_readonly(false);
            std::fs::set_permissions(&path, dir_perms)?;
        } else {
            if let Some(p) = path.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                    let mut parent_perms = std::fs::metadata(p)?.permissions();
                    parent_perms.set_readonly(false);
                    std::fs::set_permissions(p, parent_perms)?;
                }
            }
            
            entry.unpack(&path)?;
            let mut file_perms = std::fs::metadata(&path)?.permissions();
            file_perms.set_readonly(false);
            std::fs::set_permissions(&path, file_perms)?;
        }
    }
    
    Ok(())
}
