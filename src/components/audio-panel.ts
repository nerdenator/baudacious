/** Audio panel — populates device dropdowns, wires start/stop audio stream */

import { listAudioDevices, startAudioStream, stopAudioStream } from '../services/backend-api';
import { setAudioState } from '../services/app-state';

let _resetAudio: (() => void) | null = null;
let _setSelected: ((inputId: string | null, outputId: string | null) => void) | null = null;
let _applyInputDevice: ((deviceId: string | null) => Promise<void>) | null = null;

/** Reset the audio panel to stopped state (e.g. on backend-initiated device loss) */
export function resetAudioPanel(): void {
  _resetAudio?.();
}

/** Update the sidebar audio dropdown selections without triggering a stream restart */
export function setSelectedAudioDevices(inputId: string | null, outputId: string | null): void {
  _setSelected?.(inputId, outputId);
}

/** Start streaming from the given input device (or stop if null). Used by settings Save & Apply. */
export async function applyAudioInputDevice(deviceId: string | null): Promise<void> {
  await _applyInputDevice?.(deviceId);
}

export function setupAudioPanel(): void {
  const inputDropdown = document.getElementById('audio-input') as HTMLSelectElement;
  const outputDropdown = document.getElementById('audio-output') as HTMLSelectElement;
  const audioInStatus = document.getElementById('audio-in-status');
  const audioDot = audioInStatus?.querySelector('.status-dot') as HTMLElement | null;
  const audioText = audioInStatus?.querySelector('.status-text') as HTMLElement | null;
  const audioOutStatus = document.getElementById('audio-out-status');
  const audioOutDot = audioOutStatus?.querySelector('.status-dot') as HTMLElement | null;
  const audioOutText = audioOutStatus?.querySelector('.status-text') as HTMLElement | null;
  const refreshBtn = document.getElementById('audio-refresh-btn') as HTMLButtonElement | null;

  if (!inputDropdown) return;

  let streaming = false;

  // Populate dropdowns from backend on load
  populateDropdowns(inputDropdown, outputDropdown);

  // When input device changes, start/stop audio stream
  inputDropdown.addEventListener('change', async () => {
    const deviceId = inputDropdown.value;

    // Stop any existing stream first
    if (streaming) {
      try {
        await stopAudioStream();
      } catch (err) {
        console.error('Failed to stop audio stream:', err);
      }
      streaming = false;
      setAudioState(false, null);
    }

    if (!deviceId) {
      // Empty selection — reset status
      setStatus('disconnected', 'N/C');
      return;
    }

    // Start streaming from the selected device
    const deviceName = inputDropdown.options[inputDropdown.selectedIndex]?.text ?? deviceId;
    try {
      await startAudioStream(deviceId);
      streaming = true;
      setStatus('connected', 'OK');
      setAudioState(true, deviceName);
    } catch (err) {
      console.error('Failed to start audio stream:', err);
      setStatus('disconnected', 'Error');
      setAudioState(false, null);
    }
  });

  function setStatus(state: 'connected' | 'disconnected', text: string): void {
    if (audioDot) {
      audioDot.classList.remove('connected', 'disconnected');
      audioDot.classList.add(state);
    }
    if (audioText) {
      audioText.classList.remove('connected', 'disconnected');
      audioText.classList.add(state);
      audioText.textContent = text;
    }
  }

  function setOutputStatus(state: 'connected' | 'disconnected', text: string): void {
    if (audioOutDot) {
      audioOutDot.classList.remove('connected', 'disconnected');
      audioOutDot.classList.add(state);
    }
    if (audioOutText) {
      audioOutText.classList.remove('connected', 'disconnected');
      audioOutText.classList.add(state);
      audioOutText.textContent = text;
    }
  }

  // When output device changes, update the Audio Out status indicator
  outputDropdown?.addEventListener('change', () => {
    if (outputDropdown.value) {
      setOutputStatus('connected', 'OK');
    } else {
      setOutputStatus('disconnected', 'N/C');
    }
  });

  function resetAudio(): void {
    streaming = false;
    setAudioState(false, null);
    setStatus('disconnected', 'N/C');
    setOutputStatus('disconnected', 'N/C');
    inputDropdown.value = '';
    if (outputDropdown) outputDropdown.value = '';
  }

  // Refresh button — re-enumerates devices without restarting the app
  refreshBtn?.addEventListener('click', () => {
    populateDropdowns(inputDropdown, outputDropdown);
  });

  _resetAudio = resetAudio;
  _setSelected = (inputId, outputId) => {
    if (inputId !== null) inputDropdown.value = inputId;
    if (outputId !== null && outputDropdown) {
      outputDropdown.value = outputId;
      setOutputStatus(outputId ? 'connected' : 'disconnected', outputId ? 'OK' : 'N/C');
    }
  };
  _applyInputDevice = async (deviceId) => {
    // Stop any existing stream first
    if (streaming) {
      try { await stopAudioStream(); } catch { /* ignore */ }
      streaming = false;
      setAudioState(false, null);
    }
    if (!deviceId) {
      inputDropdown.value = '';
      setStatus('disconnected', 'N/C');
      return;
    }
    inputDropdown.value = deviceId;
    const deviceName = inputDropdown.options[inputDropdown.selectedIndex]?.text ?? deviceId;
    try {
      await startAudioStream(deviceId);
      streaming = true;
      setStatus('connected', 'OK');
      setAudioState(true, deviceName);
    } catch (err) {
      console.error('Failed to start audio stream:', err);
      setStatus('disconnected', 'Error');
      setAudioState(false, null);
    }
  };
}

/** Fetch audio devices from backend and populate both dropdowns */
async function populateDropdowns(
  inputDropdown: HTMLSelectElement,
  outputDropdown: HTMLSelectElement | null,
): Promise<void> {
  try {
    const devices = await listAudioDevices();

    // Clear existing options except the placeholder
    while (inputDropdown.options.length > 1) {
      inputDropdown.remove(1);
    }
    if (outputDropdown) {
      while (outputDropdown.options.length > 1) {
        outputDropdown.remove(1);
      }
    }

    for (const device of devices) {
      if (device.isInput) {
        const option = document.createElement('option');
        option.value = device.id;
        option.textContent = device.name + (device.isDefault ? ' (Default)' : '');
        inputDropdown.appendChild(option);
      }
    }

    if (outputDropdown) {
      const confirmed = devices.filter(d => d.isOutput && !d.outputUnverified);
      const unverified = devices.filter(d => d.isOutput && d.outputUnverified);

      if (unverified.length > 0) {
        const confirmedGroup = document.createElement('optgroup');
        confirmedGroup.label = 'Output Devices';
        for (const device of confirmed) {
          const opt = document.createElement('option');
          opt.value = device.id;
          opt.textContent = device.name + (device.isDefault ? ' (Default)' : '');
          confirmedGroup.appendChild(opt);
        }
        outputDropdown.appendChild(confirmedGroup);

        const unverifiedGroup = document.createElement('optgroup');
        unverifiedGroup.label = 'Other Devices';
        for (const device of unverified) {
          const opt = document.createElement('option');
          opt.value = device.id;
          opt.textContent = device.name + (device.isDefault ? ' (Default)' : '');
          unverifiedGroup.appendChild(opt);
        }
        outputDropdown.appendChild(unverifiedGroup);
      } else {
        for (const device of confirmed) {
          const opt = document.createElement('option');
          opt.value = device.id;
          opt.textContent = device.name + (device.isDefault ? ' (Default)' : '');
          outputDropdown.appendChild(opt);
        }
      }
    }
  } catch (err) {
    console.error('Failed to list audio devices:', err);
  }
}
