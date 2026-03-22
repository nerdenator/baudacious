# Plan: TX State Machine

## Problem

`start_tx` and `start_tune` currently hold the `tx_thread` mutex guard from the
"am I already running?" check all the way through to the JoinHandle store — a
span that includes potentially slow work:

- config reads (additional mutex acquisitions)
- PSK-31 encoding (CPU-bound, small but nonzero)
- `with_radio` calls (serial I/O — mode check, TX power set)

The guard is held that entire time to close the TOCTOU window: if we released it
after the check, a concurrent `start_tx` could pass the guard test and overwrite
the handle we haven't stored yet.

The side effect: `stop_tx`/`stop_tune` cannot acquire `tx_thread` during this
window, so a user's abort request is silently delayed until the serial round-trip
completes (typically 50–150 ms for CAT, potentially longer on a slow/noisy port).

## Solution

Replace the single `tx_thread: Mutex<Option<JoinHandle<()>>>` field with a
`tx_state: Mutex<TxState>` that has three variants:

```
Idle      — no TX, can start
Starting  — claimed by a start call, slow work in progress, not yet spawned
Running(JoinHandle<()>)  — thread live, handle stored
```

The lock is held for only two brief critical sections per start call:

1. **Claim**: `Idle → Starting` (check-then-transition, no I/O)
2. **Activate**: `Starting → Running(handle)` (store handle after spawn)

Between those two sections the lock is free — `stop_tx` can run immediately and
set `tx_abort = true`. `start_tx` checks `tx_abort` after the slow work; if it
is set, it transitions back to `Idle` without spawning and returns an error.

## State Transitions

```
              start_tx/start_tune (claim)
Idle ─────────────────────────────────► Starting
                                            │
              [slow work: encode, with_radio]
                                            │
              start_tx/start_tune (activate)│
Starting ───────────────────────────────────► Running
    ▲                │ (tx_abort set by         │
    │                │  concurrent stop)         │
    │         start_tx: abort detected,          │
    │         returns Err("Cancelled")           │
    │                │                           │
    └────────────────┘           stop_tx/stop_tune
                                 (take handle → Idle)
                                                 │
                                 run_tx_thread completes normally
                                 (self-clear via try_lock → Idle)
```

## Changes Required

### 1. `state.rs` — New `TxState` enum + field rename

Define the enum in `state.rs` (or a new `src/tx_state.rs` if preferred):

```rust
pub enum TxState {
    Idle,
    Starting,
    Running(std::thread::JoinHandle<()>),
}
```

In `AppState`:
- Remove: `tx_thread: Mutex<Option<JoinHandle<()>>>`
- Add:    `tx_state: Mutex<TxState>`
- `tx_abort: Arc<AtomicBool>` stays unchanged

`AppState::new()` initialises `tx_state` to `TxState::Idle`.

### 2. `commands/tx.rs` — Refactor `start_tx` and `start_tune`

**New `start_tx` skeleton:**

```rust
pub fn start_tx(...) -> Result<(), String> {
    // ── Critical section 1: claim the TX slot ──────────────────────────────
    {
        let mut tx = state.tx_state.lock().map_err(|_| "TX state corrupted")?;
        match *tx {
            TxState::Idle => *tx = TxState::Starting,
            _ => return Err("Already transmitting".into()),
        }
    }
    // ── Slow work (lock-free) ───────────────────────────────────────────────
    let carrier_freq = state.config.lock().unwrap().carrier_freq;
    let sample_rate  = state.config.lock().unwrap().sample_rate;

    let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
    let samples = encoder.encode(&text);
    if samples.is_empty() {
        *state.tx_state.lock().map_err(|_| "TX state corrupted")? = TxState::Idle;
        return Err("Nothing to transmit".into());
    }

    state.tx_abort.store(false, Ordering::SeqCst);

    let _ = with_radio(&state, &app, |radio| {
        ensure_data_mode(radio.as_mut());
        // set TX power ...
        Ok(())
    });

    // Did a concurrent stop_tx arrive during slow work?
    if state.tx_abort.load(Ordering::SeqCst) {
        *state.tx_state.lock().map_err(|_| "TX state corrupted")? = TxState::Idle;
        return Err("Cancelled before transmitting".into());
    }

    // ── Spawn ───────────────────────────────────────────────────────────────
    let handle = thread::spawn(move || { run_tx_thread(...); });

    // ── Critical section 2: activate ───────────────────────────────────────
    *state.tx_state.lock().map_err(|_| "TX state corrupted")? = TxState::Running(handle);

    Ok(())
}
```

Apply the same pattern to `start_tune`.

### 3. `commands/tx.rs` — Refactor `stop_tx_inner` and `stop_tune_inner`

```rust
fn stop_tx_inner(state: &AppState) -> Result<(), String> {
    let handle = {
        let mut tx = state.tx_state.lock().map_err(|_| "TX state corrupted")?;
        state.tx_abort.store(true, Ordering::SeqCst);
        match std::mem::replace(&mut *tx, TxState::Idle) {
            TxState::Running(h) => Some(h),
            _ => None,  // Idle or Starting — abort flag is enough
        }
    }; // lock released before join

    if let Some(handle) = handle {
        handle.join().map_err(|_| "TX thread panicked")?;
    }

    // belt-and-suspenders PTT OFF ...
    Ok(())
}
```

Apply the same pattern to `stop_tune_inner` (which also restores TX power).

### 4. `commands/tx.rs` — Update `run_tx_thread` self-clear

At the end of normal playback, `run_tx_thread` currently does:
```rust
if let Ok(mut guard) = radio_state.tx_thread.try_lock() {
    let _ = guard.take();
}
```

Replace with:
```rust
if let Ok(mut tx) = radio_state.tx_state.try_lock() {
    if matches!(*tx, TxState::Running(_)) {
        *tx = TxState::Idle;
    }
}
```

### 5. Remove `validate_tx_start`

`validate_tx_start(bool)` becomes unnecessary — the state machine check is now
inline in the match. Remove the function and its tests; the same invariant is now
enforced structurally by the enum match.

Alternatively, keep it as `validate_tx_state(state: &TxState) -> Result<(), String>`
if the unit-testable helper pattern is still wanted.

## Tests

Each new behaviour needs a test in `commands/tx.rs`:

| Test | What it covers |
|------|---------------|
| `tx_state_starts_idle` | `AppState::new()` initialises to `TxState::Idle` |
| `start_tx_transitions_to_starting_then_running` | full happy path with mock state |
| `start_tx_rejects_when_starting` | concurrent start returns `Err("Already transmitting")` while state is `Starting` |
| `start_tx_rejects_when_running` | same, state is `Running(_)` |
| `start_tx_aborted_during_slow_work` | pre-set `tx_abort=true` before critical section 2; state returns to `Idle` |
| `stop_tx_during_starting_sets_abort_no_join` | state is `Starting`; stop sets abort, returns `Ok`, state is `Idle` |
| `stop_tx_during_running_joins_handle` | state is `Running(handle)`; stop joins and clears to `Idle` |
| `stop_tx_when_idle_is_ok` | no-op, no panic |
| Existing `stop_tx_inner_*` tests | update to use new state shape |

## Out of Scope

- Audio thread (`audio_thread: Mutex<Option<JoinHandle<()>>>`) uses the same
  `Option<JoinHandle>` pattern but does not have the same serial-I/O-under-lock
  problem. No change needed there.

## Branch

Implement on a new branch off `main` (not `improve-test-coverage`, which is
pending merge).
