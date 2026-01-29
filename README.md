# Iceoryx2 Camera Synchronization Demo

This demo showcases **hardware camera synchronization** using Iceoryx2 pub/sub with QoS settings. It solves the real-world problem of synchronizing hardware trigger timestamps with V4L2 camera frames.

## Key Improvements (Latest Version)

- ✅ **Fixed critical timestamp correlation bug** for high-frequency triggers
- ✅ **Bidirectional correlation algorithm** prefers past triggers over future ones
- ✅ **Automatic trigger cleanup** prevents memory bloat and maintains performance
- ✅ **Optimized for 30fps cameras** with 110ms V4L2 delay (33ms trigger intervals)
- ✅ **Enhanced logging** shows trigger type [PAST/FUTURE], match score, and cleanup count

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

**IMPROVED: Bidirectional correlation with past trigger preference**

When a V4L2 frame arrives, the subscriber finds the best matching trigger using timestamp proximity, preferring past triggers over future ones:

1. **Frame Arrival**: V4L2 delivers frame with its own timestamp (`v4l2_ts`)
2. **Trigger Search**: Scan buffered triggers within ±500ms window
3. **Past Preference**: Prioritize triggers where `hw_ts < v4l2_ts` (no penalty)
4. **Future Penalty**: Penalize triggers where `hw_ts > v4l2_ts` (2x scoring penalty)
5. **Best Match**: Select trigger with lowest score (time difference)
6. **Cleanup**: Remove all older triggers (they'll never be useful for future frames)

**Why prefer past triggers?**
- Hardware timestamp = actual exposure time (when shutter opened)
- V4L2 timestamp = delivery time (when frame arrived in memory)
- Past triggers are more likely to be the correct exposure trigger
- Future triggers might be from subsequent frames (wrong correlation)

**Example Correlation** (30fps camera, 110ms V4L2 delay):
```
V4L2 Frame: v4l2_ts = 1150ms (delayed delivery)
Available Triggers:
  - Trigger A: hw_ts = 1017ms (past, 133ms difference, score = 133ms)
  - Trigger B: hw_ts = 1050ms (past, 100ms difference, score = 100ms) ← BEST MATCH
  - Trigger C: hw_ts = 1183ms (future, 33ms difference, score = 66ms, but penalized)

Result: Frame matches Trigger B (hw_ts = 1050ms) - accurate exposure time!
```

**Critical Fix**: Original algorithm only looked for future triggers, which fails when V4L2 delay > trigger interval (e.g., 110ms delay with 33ms intervals creates 3+ future candidates, causing wrong matches).

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
- **Trigger Type**: [PAST] or [FUTURE] indicating correlation preference
- **Match Score**: Time difference used for correlation (lower = better)
- **Cleanup Count**: Number of old triggers removed after successful match

### Memory Management
**Automatic Trigger Cleanup**: After successful frame-trigger matching, all older triggers are removed from the buffer. This prevents memory bloat and maintains performance:

- **Why it works**: Future frames arrive later and need newer triggers
- **Benefit**: Bounded memory usage, faster correlation searches
- **For 30fps cameras**: Cleans up ~3 old triggers per frame match

## Running the Camera Sync Demo

**Publisher (Trigger Source)**:
```bash
# Default 30 FPS (33ms intervals)
cargo run --bin publisher

# Custom trigger interval (e.g., 60 FPS = 16.7ms ≈ 17ms)
cargo run --bin publisher 17

# High-speed camera (500 FPS = 2ms intervals)
cargo run --bin publisher 2
```

**Subscriber (V4L2 Camera)**:
```bash
# Default V4L2 delay (150ms)
cargo run --bin subscriber

# Your specific setup: 30fps camera with 110ms V4L2 delay
cargo run --bin subscriber 110

# Fast processing camera (50ms V4L2 delay)
cargo run --bin subscriber 50
```

**Example for your 30fps camera**:
```bash
# Terminal 1: Publisher with 33ms intervals (30fps)
cargo run --bin publisher 33

# Terminal 2: Subscriber with 110ms V4L2 delay
cargo run --bin subscriber 110
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
SYNCED [PAST]: trigger_id=57, hw_exposure_ts=1769704462618433000, v4l2_ts=1769704462778936000, total_latency=160.5ms, v4l2_delay=160.5ms, score=11.2ms, cleaned=3
CLEANUP: Removed old trigger id=54 (too old for future frames)
CLEANUP: Removed old trigger id=55 (too old for future frames)
CLEANUP: Removed old trigger id=56 (too old for future frames)
SYNCED [PAST]: trigger_id=61, hw_exposure_ts=1769704462763919000, v4l2_ts=1769704462946580000, total_latency=182.7ms, v4l2_delay=182.7ms, score=8.9ms, cleaned=2
...
```

**Late-Joining Subscriber Example** (starts after 1000+ triggers):
```
Camera sync subscriber started. Synchronizing hardware timestamps with V4L2 frames...
Draining historical triggers...
Drained 0 historical triggers. Starting real-time sync...
Received trigger: id=1047, hw_ts=1769704562251947000, ipc_delay=1000ns
SYNCED [PAST]: trigger_id=1057, hw_exposure_ts=1769704562618433000, v4l2_ts=1769704562778936000, total_latency=160.5ms, v4l2_delay=160.5ms, score=11.2ms, cleaned=3
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