# Plan: Waterfall Performance

## Problem

The waterfall rendering has a noticeable stutter, particularly visible at the
end of a TX cycle. We've made incremental improvements (RAF batching, drawImage
scroll, single-row putImageData) but the jank persists.

## Known Root Cause Candidates

### 1. JS main thread — still doing too much per frame
Even with RAF batching, `flushPendingRows` runs on the main thread and calls
`drawImage` + `putImageData` in a loop for every queued row. If multiple rows
queue up (e.g. after a burst), this loop does N × (GPU copy + row write) in a
single frame, potentially exceeding 16ms.

### 2. `fft-data` events arriving at full Rust rate, no throttle
The Rust audio thread emits an FFT event every ~43ms (~23fps). Each event wakes
the JS event loop, queues a row, and schedules a RAF. If the JS frame rate
drops below 23fps (which it will under load), rows pile up and a burst flush
occurs on recovery — visible as a "catch-up" lurch.

### 3. Audio device reconfiguration on TX end
When `CpalAudioOutput` is dropped at TX completion, CoreAudio briefly
reconfigures the USB audio device. This can stall the input stream, creating a
gap followed by a burst of buffered FFT events — exactly matching the TX-end
hitch pattern.

### 4. OffscreenCanvas not used
The waterfall canvas runs on the main thread. Moving it to an
`OffscreenCanvas` + `Worker` would isolate all rendering from the JS main
thread entirely, eliminating jank from DOM work, IPC callbacks, and event
processing.

---

## Proposed Fix: OffscreenCanvas Worker

Move all waterfall rendering into a `Worker` using `OffscreenCanvas`. The
worker owns the canvas, receives FFT data via `postMessage`, and renders
continuously without competing with the main thread.

### Architecture

```
Main thread                       Worker (waterfall-worker.ts)
──────────────────────────────    ──────────────────────────────
fft-data event received           ← postMessage({ type: 'row', magnitudes })
                                    queues row
                                    RAF loop: flush all queued rows
                                    drawImage scroll + putImageData

settings change (palette etc.)   ← postMessage({ type: 'settings', ... })
carrier freq change               ← postMessage({ type: 'carrier', freqHz })
canvas resize                     ← postMessage({ type: 'resize', w, h })
```

### Files to Change

1. **`src/components/waterfall.ts`** — strip rendering logic; expose
   `transferToOffscreen()` and a `postMessage` wrapper
2. **`src/workers/waterfall-worker.ts`** (new) — all canvas operations,
   color maps, scroll logic, RAF loop
3. **`src/services/audio-bridge.ts`** — `postMessage` to worker instead of
   calling `waterfall.drawSpectrum()`
4. **`index.html`** — no changes needed

### Fallback
If `OffscreenCanvas` is unavailable (old WebKit), fall back to the current
main-thread approach. Tauri's WebKit on macOS has supported OffscreenCanvas
since macOS 13.

---

## Simpler Short-Term Alternative (if OffscreenCanvas is too much work)

Cap rows flushed per RAF frame to 1 and drop the rest:
```typescript
private flushPendingRows(): void {
  const rows = this.pendingRows.splice(-1); // only render the newest row
  this.pendingRows = [];
  // render rows[0]...
}
```
This keeps the waterfall smooth at the cost of occasionally skipping rows
during bursts. For a scrolling spectrogram, dropping intermediate rows is
nearly invisible.

---

## Tests

- Playwright: waterfall canvas receives and renders FFT data (existing test
  should still pass after refactor)
- Manual: no stutter visible during TX→RX transition
- Manual: smooth scroll at 23fps during sustained RX
