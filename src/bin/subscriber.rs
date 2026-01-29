use iceoryx2::prelude::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::collections::VecDeque;
use std::env;

// Use tuple: (frame_id, hardware_timestamp_ns, publish_timestamp_ns)
type CameraTrigger = (u64, u64, u64);

#[derive(Debug)]
struct V4L2Frame {
    frame_id: u64,
    v4l2_timestamp_ns: u64,  // When V4L2 delivered the frame
    data: Vec<u8>,  // Simulated frame data
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let v4l2_delay_ms = if args.len() > 1 {
        args[1].parse::<u64>().unwrap_or(150)
    } else {
        150 // Default V4L2 delay in milliseconds
    };

    println!("Camera sync subscriber started with V4L2 delay: {}ms", v4l2_delay_ms);
    println!("Usage: {} [v4l2_delay_ms]", args[0]);
    println!("Synchronizing hardware timestamps with V4L2 frames...");

    let node = NodeBuilder::new().create::<ipc::Service>()?;

    // Open the same service
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

    let subscriber = service
        .subscriber_builder()
        .create()?;

    println!("Camera sync subscriber started. Synchronizing hardware timestamps with V4L2 frames...");

    // Buffer for pending triggers waiting for V4L2 frames
    let mut pending_triggers: VecDeque<CameraTrigger> = VecDeque::new();

    // Drain historical triggers at the beginning (if any)
    println!("Draining historical triggers...");
    let mut history_count = 0;
    while let Some(trigger) = subscriber.receive()? {
        let (trigger_id, hw_ts, _pub_ts) = *trigger;
        println!("Historical trigger: id={}, hw_ts={}", trigger_id, hw_ts);
        pending_triggers.push_back(*trigger);
        history_count += 1;
    }
    println!("Drained {} historical triggers. Starting real-time sync...", history_count);

    loop {
        // Receive new triggers
        while let Some(trigger) = subscriber.receive()? {
            let (trigger_id, hw_ts, pub_ts) = *trigger;
            println!("Received trigger: id={}, hw_ts={}, ipc_delay={}ns",
                     trigger_id, hw_ts, pub_ts.saturating_sub(hw_ts));

            pending_triggers.push_back(*trigger);

            // Limit pending triggers to avoid memory issues (keep last 100)
            if pending_triggers.len() > 100 {
                if let Some((old_trigger_id, _, _)) = pending_triggers.pop_front() {
                    println!("WARNING: Dropped old trigger id={} (V4L2 too slow)", old_trigger_id);
                }
            }
        }

        // Simulate V4L2 frame capture (slower than triggers)
        // In real code, this would be your V4L2 capture loop
        if !pending_triggers.is_empty() {
            // Simulate V4L2 processing delay (configurable via command line)
            std::thread::sleep(Duration::from_millis(v4l2_delay_ms));

            // Simulate receiving a frame from V4L2
            let v4l2_timestamp_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_nanos() as u64;

            // Find the best matching trigger based on timestamp proximity
            // IMPROVED: Handle case where V4L2 delay > trigger interval
            // Prefer past triggers (hw_ts < v4l2_ts) but allow future triggers as fallback
            let mut best_match_index = None;
            let mut best_score = f64::MAX;

            for (index, (_trigger_id, hw_ts, _pub_ts)) in pending_triggers.iter().enumerate() {
                let time_diff_ns = if v4l2_timestamp_ns > *hw_ts {
                    v4l2_timestamp_ns - hw_ts
                } else {
                    hw_ts - v4l2_timestamp_ns
                };
                let time_diff_ms = time_diff_ns as f64 / 1_000_000.0;

                // Prefer past triggers (hw_ts < v4l2_ts) - these are more likely correct
                // Penalize future triggers since they might be from subsequent frames
                let is_past_trigger = *hw_ts < v4l2_timestamp_ns;
                let score = if is_past_trigger {
                    time_diff_ms  // No penalty for past triggers
                } else {
                    time_diff_ms * 2.0  // 2x penalty for future triggers
                };

                // Allow up to 500ms tolerance for matching (adjust based on your system)
                if score < best_score && time_diff_ms < 500.0 {
                    best_score = score;
                    best_match_index = Some(index);
                }
            }

            if let Some(match_index) = best_match_index {
                let (trigger_id, hw_ts, pub_ts) = pending_triggers.remove(match_index).unwrap();

                // OPTIMIZATION: Remove all triggers older than the matched one
                // These will never be useful for future frames since they're too old
                let removed_old_count = match_index; // Number of triggers before the matched one
                for _ in 0..removed_old_count {
                    if let Some((old_trigger_id, _, _)) = pending_triggers.pop_front() {
                        println!("CLEANUP: Removed old trigger id={} (too old for future frames)", old_trigger_id);
                    }
                }

                // Calculate synchronization metrics
                let total_latency_ms = (v4l2_timestamp_ns - hw_ts) as f64 / 1_000_000.0;
                let v4l2_delay_ms = (v4l2_timestamp_ns - pub_ts) as f64 / 1_000_000.0;
                let trigger_type = if hw_ts < v4l2_timestamp_ns { "PAST" } else { "FUTURE" };

                println!("SYNCED [{}]: trigger_id={}, hw_exposure_ts={}, v4l2_ts={}, total_latency={:.1}ms, v4l2_delay={:.1}ms, score={:.1}ms, cleaned={}",
                         trigger_type, trigger_id, hw_ts, v4l2_timestamp_ns, total_latency_ms, v4l2_delay_ms, best_score, removed_old_count);

                // Process the synchronized frame here
                // Your frame processing code would go here

            } else {
                // No suitable trigger found within tolerance
                println!("WARNING: V4L2 frame at {}ns - no matching trigger within 500ms tolerance", v4l2_timestamp_ns);
            }
        }

        // Small delay to prevent busy waiting
        std::thread::sleep(Duration::from_millis(10));
    }
}