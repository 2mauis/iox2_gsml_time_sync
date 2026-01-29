use iceoryx2::prelude::*;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType, Resolution};
use nokhwa::Camera;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::VecDeque;
use std::env;
use eframe::egui;
use eframe::egui::{ColorImage, TextureHandle};

// Use tuple: (frame_id, hardware_timestamp_ns, publish_timestamp_ns)
type CameraTrigger = (u64, u64, u64);

#[derive(Default)]
struct CameraApp {
    camera: Option<Camera>,
    subscriber: Option<iceoryx2::port::subscriber::Subscriber<iceoryx2::service::ipc::Service, CameraTrigger, ()>>,
    pending_triggers: VecDeque<CameraTrigger>,
    trigger_count: u32,
    skip_ratio: u32,
    output_fps: u32,
    camera_index: u32,
    width: u32,
    height: u32,
    current_frame: Option<ColorImage>,
    texture: Option<TextureHandle>,
    sync_info: String,
    is_running: bool,
}

impl CameraApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Parse command line arguments
        let args: Vec<String> = env::args().collect();

        // Default values
        let mut camera_index = 0u32;
        let mut output_fps = 30u32;
        let mut width = 640u32;
        let mut height = 480u32;

        // Parse arguments: v4l2_capture [camera_index] [output_fps] [width] [height]
        if args.len() > 1 {
            if let Ok(idx) = args[1].parse::<u32>() {
                camera_index = idx;
            }
        }
        if args.len() > 2 {
            if let Ok(fps) = args[2].parse::<u32>() {
                output_fps = fps;
            }
        }
        if args.len() > 3 {
            if let Ok(w) = args[3].parse::<u32>() {
                width = w;
            }
        }
        if args.len() > 4 {
            if let Ok(h) = args[4].parse::<u32>() {
                height = h;
            }
        }

        // Calculate frame skip ratio
        let input_fps = 30u32;
        let skip_ratio = if output_fps >= input_fps {
            1
        } else {
            (input_fps as f32 / output_fps as f32).round() as u32
        };

        let mut app = Self {
            camera: None,
            subscriber: None,
            pending_triggers: VecDeque::new(),
            trigger_count: 0,
            skip_ratio,
            output_fps,
            camera_index,
            width,
            height,
            current_frame: None,
            texture: None,
            sync_info: "Initializing...".to_string(),
            is_running: false,
        };

        // Initialize camera and Iceoryx2
        if let Err(e) = app.initialize() {
            app.sync_info = format!("Initialization error: {}", e);
        }

        app
    }

    fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.sync_info = format!("Initializing camera {} and Iceoryx2 sync...", self.camera_index);

        // Initialize camera
        let camera_index = CameraIndex::Index(self.camera_index);
        let requested_format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
        let mut camera = Camera::new(camera_index, requested_format)?;
        camera.set_resolution(Resolution::new(self.width, self.height))?;
        camera.open_stream()?;
        self.camera = Some(camera);

        // Initialize Iceoryx2 subscriber
        let node = NodeBuilder::new().create::<ipc::Service>()?;
        let service = node
            .service_builder(&"Camera/Sync".try_into()?)
            .publish_subscribe::<CameraTrigger>()
            .enable_safe_overflow(true)
            .history_size(10)
            .subscriber_max_buffer_size(20)
            .max_subscribers(3)
            .max_publishers(1)
            .open_or_create()?;

        let subscriber = service.subscriber_builder().create()?;
        self.subscriber = Some(subscriber);

        // Drain historical triggers
        self.sync_info = "Draining historical triggers...".to_string();
        let mut history_count = 0;
        if let Some(subscriber) = &self.subscriber {
            while let Some(_) = subscriber.receive()? {
                history_count += 1;
            }
        }
        self.sync_info = format!("Ready! Drained {} historical triggers. Click 'Start Capture' to begin.", history_count);
        Ok(())
    }

    fn capture_frame(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(camera) = &mut self.camera {
            // Receive new triggers
            if let Some(subscriber) = &self.subscriber {
                while let Some(trigger) = subscriber.receive()? {
                    let (trigger_id, hw_ts, pub_ts) = *trigger;
                    println!("Received trigger: id={}, hw_ts={}, ipc_delay={}ns",
                             trigger_id, hw_ts, pub_ts.saturating_sub(hw_ts));
                    self.pending_triggers.push_back(*trigger);

                    // Limit pending triggers
                    if self.pending_triggers.len() > 100 {
                        if let Some((old_trigger_id, _, _)) = self.pending_triggers.pop_front() {
                            println!("WARNING: Dropped old trigger id={} (V4L2 too slow)", old_trigger_id);
                        }
                    }
                }
            }

            // Capture frame
            let frame = camera.frame()?;
            let v4l2_timestamp_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64;

            // Frame skipping
            self.trigger_count += 1;
            let should_process = (self.trigger_count % self.skip_ratio) == 0;

            if should_process {
                // Synchronize with trigger
                self.sync_frame_with_trigger(&frame, v4l2_timestamp_ns)?;

                // Convert frame to ColorImage for display
                let buffer = frame.buffer();
                let resolution = frame.resolution();
                let actual_width = resolution.width_x as usize;
                let actual_height = resolution.height_y as usize;

                let pixels: Vec<egui::Color32> = buffer
                    .chunks_exact(3)
                    .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                    .collect();

                // Check if we got the expected number of pixels
                let expected_pixels = actual_width * actual_height;
                if pixels.len() == expected_pixels {
                    self.current_frame = Some(ColorImage {
                        size: [actual_width, actual_height],
                        pixels,
                        source_size: egui::Vec2::new(actual_width as f32, actual_height as f32),
                    });
                    // Update stored dimensions if they changed
                    self.width = actual_width as u32;
                    self.height = actual_height as u32;
                } else {
                    self.sync_info = format!("Frame size mismatch: got {} pixels, expected {} ({}x{})",
                                           pixels.len(), expected_pixels, actual_width, actual_height);
                }
            } else {
                println!("SKIPPED: Frame {} skipped (output FPS: {}fps, processing every {}th trigger)",
                         self.trigger_count, self.output_fps, self.skip_ratio);
            }
        }
        Ok(())
    }

    fn sync_frame_with_trigger(&mut self, frame: &nokhwa::Buffer, v4l2_timestamp_ns: u64) -> Result<(), Box<dyn std::error::Error>> {
        let mut best_match_index = None;
        let mut best_score = f64::MAX;

        for (index, (_trigger_id, hw_ts, _pub_ts)) in self.pending_triggers.iter().enumerate() {
            let time_diff_ns = if v4l2_timestamp_ns > *hw_ts {
                v4l2_timestamp_ns - hw_ts
            } else {
                hw_ts - v4l2_timestamp_ns
            };
            let time_diff_ms = time_diff_ns as f64 / 1_000_000.0;

            let is_past_trigger = *hw_ts < v4l2_timestamp_ns;
            let score = if is_past_trigger {
                time_diff_ms
            } else {
                time_diff_ms * 2.0
            };

            if score < best_score && time_diff_ms < 500.0 {
                best_score = score;
                best_match_index = Some(index);
            }
        }

        if let Some(match_index) = best_match_index {
            let (trigger_id, hw_ts, pub_ts) = self.pending_triggers.remove(match_index).unwrap();

            // Cleanup old triggers
            let removed_old_count = match_index;
            for _ in 0..removed_old_count {
                if let Some((old_trigger_id, _, _)) = self.pending_triggers.pop_front() {
                    println!("CLEANUP: Removed old trigger id={} (too old for future frames)", old_trigger_id);
                }
            }

            let total_latency_ms = (v4l2_timestamp_ns - hw_ts) as f64 / 1_000_000.0;
            let v4l2_delay_ms = (v4l2_timestamp_ns - pub_ts) as f64 / 1_000_000.0;
            let trigger_type = if hw_ts < v4l2_timestamp_ns { "PAST" } else { "FUTURE" };

            self.sync_info = format!("SYNCED [{}]: trigger_id={}, latency={:.1}ms, score={:.1}ms",
                                   trigger_type, trigger_id, total_latency_ms, best_score);

            println!("SYNCED [{}]: trigger_id={}, hw_exposure_ts={}, v4l2_ts={}, total_latency={:.1}ms, v4l2_delay={:.1}ms, score={:.1}ms, cleaned={}, frame_size={}bytes",
                     trigger_type, trigger_id, hw_ts, v4l2_timestamp_ns, total_latency_ms, v4l2_delay_ms, best_score, removed_old_count, frame.buffer().len());
        } else {
            self.sync_info = format!("WARNING: No matching trigger within 500ms (frame at {}ns)", v4l2_timestamp_ns);
            println!("WARNING: V4L2 frame at {}ns - no matching trigger within 500ms tolerance", v4l2_timestamp_ns);
        }

        Ok(())
    }
}

impl eframe::App for CameraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("V4L2 Camera Capture with Iceoryx2 Sync");

            ui.horizontal(|ui| {
                if ui.button(if self.is_running { "Stop Capture" } else { "Start Capture" }).clicked() {
                    self.is_running = !self.is_running;
                }

                ui.label(format!("Camera: {} | {}x{} | {}fps output",
                               self.camera_index, self.width, self.height, self.output_fps));
            });

            ui.separator();

            // Display sync info
            ui.label(&self.sync_info);

            // Display frame
            if let Some(frame) = &self.current_frame {
                // Check if we need to recreate the texture due to size change
                let needs_new_texture = if let Some(existing_texture) = &self.texture {
                    let current_size = existing_texture.size_vec2();
                    let frame_size = egui::Vec2::new(frame.size[0] as f32, frame.size[1] as f32);
                    current_size != frame_size
                } else {
                    true
                };

                if needs_new_texture {
                    self.texture = Some(ui.ctx().load_texture("camera_frame", frame.clone(), Default::default()));
                }

                if let Some(texture) = &mut self.texture {
                    // Update texture if we have a new frame
                    texture.set(frame.clone(), Default::default());

                    let size = texture.size_vec2();
                    ui.image((texture.id(), size));
                }
            } else {
                ui.label("No frame captured yet. Click 'Start Capture' to begin.");
            }
        });

        // Capture frames if running
        if self.is_running {
            if let Err(e) = self.capture_frame() {
                self.sync_info = format!("Capture error: {}", e);
                self.is_running = false;
            }
            // Request repaint for next frame
            ctx.request_repaint();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("V4L2 Camera Capture with Iceoryx2 Sync"),
        ..Default::default()
    };

    eframe::run_native(
        "V4L2 Camera Capture with Iceoryx2 Sync",
        options,
        Box::new(|cc| Ok(Box::new(CameraApp::new(cc)))),
    )?;

    Ok(())
}