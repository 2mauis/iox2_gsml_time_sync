# Iceoryx2 Camera Synchronization Demo

This demo showcases **hardware camera synchronization** using Iceoryx2 pub/sub with QoS settings. It solves the real-world problem of synchronizing hardware trigger timestamps with V4L2 camera frames.

## Camera Synchronization Problem

- **Hardware trigger** generates exposure timestamp (actual capture time)
- **V4L2 driver** delivers frames with significant delay (100-200ms)
- **Need**: Correlate V4L2 frames with their actual exposure timestamps

## Solution Architecture

### Publisher (Trigger Process)
- Captures hardware trigger timestamps immediately
- Publishes `(trigger_id, hardware_timestamp, publish_timestamp)` via Iceoryx2
- Uses low-latency IPC to minimize timing errors
- **Global trigger sequence** shared across all cameras

### Subscriber (V4L2 Process)
- Receives triggers via Iceoryx2 (fast)
- Buffers triggers waiting for V4L2 frames (slower)
- **Timestamp-based correlation** instead of sequential matching
- Supports **late-joining subscribers** (cameras that start after triggers began)
- Matches V4L2 frames with correct hardware timestamps using time proximity

## Synchronization Flow

```
Hardware Trigger → Iceoryx2 Publisher → Iceoryx2 IPC → Subscriber Buffer → V4L2 Frame → Synchronized Data
     ↑                    ↑                      ↑                    ↑              ↑              ↑
  Real Time          Minimal Delay          Microseconds         Buffer          100-200ms    Accurate
```

## Multi-Camera Support & Late-Joining Subscribers

**Problem Solved**: Traditional sequential frame_id matching fails when:
- Multiple cameras share the same trigger source
- Camera processes start at different times
- Late-joining subscribers miss many frame_ids

**Solution**: Timestamp-based correlation with tolerance window
- Triggers use global `trigger_id` (always incrementing)
- Subscribers correlate using **timestamp proximity** (within 500ms tolerance)
- Late cameras immediately sync with current triggers
- No dependency on sequential frame matching

### Timestamp Correlation Algorithm

When a V4L2 frame arrives, the subscriber finds the trigger with the closest hardware timestamp that is **further in the future**:

1. **Frame Arrival**: V4L2 delivers frame with its own timestamp (`v4l2_ts`)
2. **Trigger Search**: Scan buffered triggers for candidates where `hw_timestamp > v4l2_ts`
3. **Closest Match**: Select trigger with smallest `hw_timestamp - v4l2_ts` difference
4. **Tolerance Check**: Ensure difference ≤ 500ms (configurable tolerance window)
5. **Synchronization**: Associate frame with matched trigger's hardware timestamp

**Why "further in the future"?**
- Hardware timestamp represents actual exposure time (when shutter opened)
- V4L2 timestamp represents delivery time (when frame arrived in memory)
- We match frames to the trigger that came immediately after their exposure
- This accounts for V4L2 processing delay while maintaining temporal accuracy

**Example Correlation**:
```
V4L2 Frame: v4l2_ts = 1000ms
Available Triggers:
  - Trigger A: hw_ts = 850ms  (too early - before frame exposure)
  - Trigger B: hw_ts = 1020ms (closest future trigger - 20ms difference)
  - Trigger C: hw_ts = 1100ms (further away - 100ms difference)

Result: Frame matches Trigger B (hw_ts = 1020ms)
```

## QoS Settings for Camera Sync

### Service-Level QoS
- `history_size(10)`: Store recent triggers for late V4L2 frames
- `subscriber_max_buffer_size(20)`: Handle trigger bursts
- `enable_safe_overflow(true)`: Don't block on trigger floods
- `max_subscribers(3)`: Support multiple camera processes

### Synchronization Metrics
- **Hardware Timestamp**: Actual camera exposure time
- **IPC Latency**: Delay from trigger to Iceoryx2 delivery
- **V4L2 Delay**: Time from trigger to frame delivery
- **Total Latency**: End-to-end synchronization accuracy

## Running the Camera Sync Demo

1. **Trigger Publisher** (simulates hardware interrupts):
   ```bash
   cargo run --bin publisher
   ```

2. **V4L2 Subscriber** (simulates camera frame processing):
   ```bash
   cargo run --bin subscriber
   ```

## Expected Output

**Publisher (always running)**:
```
Published trigger: id=47, hw_ts=1769704462251947000, ipc_latency=1000ns
Published trigger: id=48, hw_ts=1769704462290007000, ipc_latency=1000ns
...
```

**Subscriber (can start late - timestamp correlation)**:
```
Camera sync subscriber started. Synchronizing hardware timestamps with V4L2 frames...
Draining historical triggers...
Drained 0 historical triggers. Starting real-time sync...
Received trigger: id=47, hw_ts=1769704462251947000, ipc_delay=1000ns
SYNCED: trigger_id=57, hw_exposure_ts=1769704462618433000, v4l2_ts=1769704462778936000, total_latency=160.5ms, v4l2_delay=160.5ms
SYNCED: trigger_id=61, hw_exposure_ts=1769704462763919000, v4l2_ts=1769704462946580000, total_latency=182.7ms, v4l2_delay=182.7ms
...
```

**Late-Joining Subscriber Example** (starts after 1000+ triggers):
```
Camera sync subscriber started. Synchronizing hardware timestamps with V4L2 frames...
Draining historical triggers...
Drained 0 historical triggers. Starting real-time sync...
Received trigger: id=1047, hw_ts=1769704562251947000, ipc_delay=1000ns
SYNCED: trigger_id=1057, hw_exposure_ts=1769704562618433000, v4l2_ts=1769704562778936000, total_latency=160.5ms, v4l2_delay=160.5ms
```

## Real-World Integration

### Hardware Trigger Publisher
```rust
// In your hardware interrupt handler
let hw_timestamp = get_hardware_timestamp();
publish_to_iceoryx2(frame_id, hw_timestamp);
```

### V4L2 Frame Subscriber
```rust
// In your V4L2 capture loop
let frame = v4l2_capture_frame();
let hw_timestamp = sync_with_trigger(frame.id);
process_synchronized_frame(frame, hw_timestamp);
```

## Key Benefits

- **Microsecond IPC latency** vs milliseconds of V4L2 delay
- **Reliable timestamp correlation** even with frame drops/reordering
- **Buffer management** handles V4L2 processing variability
- **Scalable** to multiple cameras/processes

This approach ensures your computer vision pipeline uses accurate exposure timestamps rather than V4L2 delivery timestamps!