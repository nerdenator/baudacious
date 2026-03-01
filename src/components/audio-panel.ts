/** Audio panel — populates device dropdowns, wires start/stop audio stream */

import { listAudioDevices, startAudioStream, stopAudioStream } from '../services/backend-api';
import { setAudioState } from '../services/app-state';

let _resetAudio: (() => void) | null = null;
let _setSelected: ((inputId: string | null, outputId: string | null) => void) | null = null;

/** Reset the audio panel to stopped state (e.g. on backend-initiated device loss) */
export function resetAudioPanel(): void {
  _resetAudio?.();
}

/** Update the sidebar audio dropdown selections without triggering a stream restart */
export function setSelectedAudioDevices(inputId: string | null, outputId: string | null): void {
  _setSelected?.(inputId, outputId);
}

export function setupAudioPanel(): void {
  const inputDropdown = document.getElementById('audio-input') as HTMLSelectElement;
  const outputDropdown = document.getElementById('audio-output') as HTMLSelectElement;
  const audioInStatus = document.getElementById('audio-in-status');
  const audioDot = audioInStatus?.querySelector('.status-dot') as HTMLElement | null;
  const audioText = audioInStatus?.querySelector('.status-text') as HTMLElement | null;
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

  function resetAudio(): void {
    streaming = false;
    setAudioState(false, null);
    setStatus('disconnected', 'N/C');
    inputDropdown.value = '';
  }

  // Refresh button — re-enumerates devices without restarting the app
  refreshBtn?.addEventListener('click', () => {
    populateDropdowns(inputDropdown, outputDropdown);
  });

  _resetAudio = resetAudio;
  _setSelected = (inputId, outputId) => {
    if (inputId !== null) inputDropdown.value = inputId;
    if (outputId !== null && outputDropdown) outputDropdown.value = outputId;
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
      const option = document.createElement('option');
      option.value = device.id;
      option.textContent = device.name;
      if (device.isDefault) {
        option.textContent += ' (Default)';
      }

      if (device.isInput) {
        inputDropdown.appendChild(option);
      } else if (outputDropdown) {
        outputDropdown.appendChild(option.cloneNode(true) as HTMLOptionElement);
      }
    }
  } catch (err) {
    console.error('Failed to list audio devices:', err);
  }
}
