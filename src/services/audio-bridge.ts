/** Audio bridge — forwards FFT events from Rust backend to the waterfall display */

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { WaterfallDisplay } from '../components/waterfall';

interface FftPayload {
  magnitudes: number[];
}

let fftUnlisten: UnlistenFn | null = null;
let statusUnlisten: UnlistenFn | null = null;

/** Start listening for FFT data events and piping them to the waterfall */
export async function startFftBridge(waterfall: WaterfallDisplay): Promise<void> {
  // Clean up any previous listener
  await stopFftBridge();

  fftUnlisten = await listen<FftPayload>('fft-data', (event) => {
    waterfall.drawSpectrum(event.payload.magnitudes);
  });
}

/** Listen for audio status changes */
export async function listenAudioStatus(
  onStatus?: (status: string) => void,
): Promise<void> {
  if (statusUnlisten) {
    statusUnlisten();
    statusUnlisten = null;
  }

  statusUnlisten = await listen<{ status: string }>('audio-status', (event) => {
    onStatus?.(event.payload.status);
  });
}

/** Stop listening for FFT events */
export async function stopFftBridge(): Promise<void> {
  if (fftUnlisten) {
    fftUnlisten();
    fftUnlisten = null;
  }
}

/** Stop listening for audio status events */
export function stopAudioStatusBridge(): void {
  if (statusUnlisten) {
    statusUnlisten();
    statusUnlisten = null;
  }
}
