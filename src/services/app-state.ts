/** Shared application state for cross-component coordination.
 *
 * Components (serial-panel, audio-panel) call setters when connection state changes.
 * The status bar subscribes to receive those updates.
 * On page load, hydrateFromBackend() seeds the state from Rust so the status bar
 * is accurate even after a webview reload while audio is still streaming.
 */

import { getConnectionStatus } from './backend-api';

interface SerialState {
  connected: boolean;
  portName: string | null;
}

interface AudioState {
  streaming: boolean;
  deviceName: string | null;
}

type SerialCallback = (connected: boolean, portName: string | null) => void;
type AudioCallback = (streaming: boolean, deviceName: string | null) => void;

let serialState: SerialState = { connected: false, portName: null };
let audioState: AudioState = { streaming: false, deviceName: null };

const serialSubscribers: SerialCallback[] = [];
const audioSubscribers: AudioCallback[] = [];

export function setSerialState(connected: boolean, portName: string | null): void {
  serialState = { connected, portName };
  for (const cb of serialSubscribers) cb(connected, portName);
}

export function setAudioState(streaming: boolean, deviceName: string | null): void {
  audioState = { streaming, deviceName };
  for (const cb of audioSubscribers) cb(streaming, deviceName);
}

export function getSerialState(): SerialState {
  return { ...serialState };
}

export function getAudioState(): AudioState {
  return { ...audioState };
}

export function onSerialChanged(cb: SerialCallback): void {
  serialSubscribers.push(cb);
}

export function onAudioChanged(cb: AudioCallback): void {
  audioSubscribers.push(cb);
}

/** Hydrate state from Rust backend — call once on startup so the status bar
 *  reflects actual hardware state even after a webview reload. */
export async function hydrateFromBackend(): Promise<void> {
  const status = await getConnectionStatus();
  setSerialState(status.serialConnected, status.serialPort);
  setAudioState(status.audioStreaming, status.audioDevice);
}
