use iceoryx2::prelude::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::env;

// Use tuple: (frame_id, hardware_timestamp_ns, publish_timestamp_ns)
type CameraTrigger = (u64, u64, u64);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let trigger_interval_ms = if args.len() > 1 {
        args[1].parse::<u64>().unwrap_or(33)
    } else {
        33 // Default trigger interval in milliseconds (30 FPS)
    };

    println!("Camera trigger publisher started with interval: {}ms", trigger_interval_ms);
    println!("Usage: {} [trigger_interval_ms]", args[0]);
    println!("Publishing hardware timestamps for multiple cameras...");

    let node = NodeBuilder::new().create::<ipc::Service>()?;

    // Create service with QoS settings optimized for camera sync
    let service = node
        .service_builder(&"Camera/Sync".try_into()?)
        .publish_subscribe::<CameraTrigger>()
        // Enable safe overflow for burst triggers
        .enable_safe_overflow(true)
        // Store recent triggers for late V4L2 frames
        .history_size(10)
        // Buffer for trigger bursts
        .subscriber_max_buffer_size(20)
        // Allow multiple camera processes
        .max_subscribers(3)
        // Single trigger publisher
        .max_publishers(1)
        .open_or_create()?;

    let publisher = service
        .publisher_builder()
        .max_loaned_samples(5)  // Handle trigger bursts
        .unable_to_deliver_strategy(UnableToDeliverStrategy::DiscardSample)
        .create()?;

    let mut global_trigger_id = 0;
    println!("Camera trigger publisher started. Publishing hardware timestamps for multiple cameras...");

    loop {
        // Simulate hardware trigger interrupt (shared by all cameras)
        global_trigger_id += 1;

        // Capture hardware timestamp (actual exposure time - same for all cameras)
        let hardware_timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos() as u64;

        // Publish immediately via Iceoryx2
        let publish_timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos() as u64;

        let trigger = (global_trigger_id, hardware_timestamp_ns, publish_timestamp_ns);

        let sample = publisher.loan_uninit()?;
        let sample = sample.write_payload(trigger);
        sample.send()?;

        println!("Published trigger: id={}, hw_ts={}, ipc_latency={}ns",
                 global_trigger_id,
                 hardware_timestamp_ns,
                 publish_timestamp_ns.saturating_sub(hardware_timestamp_ns));

        // Simulate configurable trigger rate
        std::thread::sleep(Duration::from_millis(trigger_interval_ms));
    }
}