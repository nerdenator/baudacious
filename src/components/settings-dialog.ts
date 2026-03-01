/** Settings dialog — tabbed configuration panel (General / Audio / Radio) */

import {
  listAudioDevices,
  listConfigurations,
  loadConfiguration,
  deleteConfiguration,
} from '../services/backend-api';
import type { Configuration, AudioDeviceInfo } from '../types';

export interface SettingsDialogDeps {
  getCurrentConfig: () => Configuration | null;
  onSave: (config: Configuration) => Promise<void>;
}

type Tab = 'general' | 'audio' | 'radio';

let _openDialog: ((tab?: Tab) => void) | null = null;

export function openSettingsDialog(tab?: Tab): void {
  _openDialog?.(tab);
}

export function setupSettingsDialog(deps: SettingsDialogDeps): void {
  // ── DOM skeleton ──────────────────────────────────────────────────────────
  const overlay = el('div', 'settings-overlay');
  const dialog = el('div', 'settings-dialog');
  overlay.appendChild(dialog);

  // Header
  const header = el('div', 'settings-header');
  const titleEl = el('span', 'settings-title');
  titleEl.textContent = 'Settings';
  const closeBtn = btn('settings-close', '✕');
  closeBtn.setAttribute('aria-label', 'Close settings');
  header.append(titleEl, closeBtn);
  dialog.appendChild(header);

  // Body: side-tab nav + panel area
  const body = el('div', 'settings-body');
  const tabNav = el('nav', 'settings-tabs');
  const panelArea = el('div', 'settings-panels');
  body.append(tabNav, panelArea);
  dialog.appendChild(body);

  // ── Tab buttons ───────────────────────────────────────────────────────────
  const TABS: Tab[] = ['general', 'audio', 'radio'];
  const TAB_LABELS: Record<Tab, string> = { general: 'General', audio: 'Audio', radio: 'Radio' };
  const tabBtns = {} as Record<Tab, HTMLButtonElement>;
  for (const t of TABS) {
    const b = btn('settings-tab', TAB_LABELS[t]);
    b.dataset['tab'] = t;
    tabBtns[t] = b;
    tabNav.appendChild(b);
  }

  // ── General panel ─────────────────────────────────────────────────────────
  const generalPanel = el('section', 'settings-panel');
  generalPanel.appendChild(sectionLabel('Profile'));

  const profileSelect = select('settings-profile-select device-select');
  generalPanel.append(deviceGroup('Load profile', profileSelect));

  const profileNameInput = document.createElement('input');
  profileNameInput.type = 'text';
  profileNameInput.className = 'settings-input';
  profileNameInput.placeholder = 'Profile name...';
  generalPanel.append(deviceGroup('Save as', profileNameInput));

  const profileActions = el('div', 'settings-profile-actions');
  const deleteBtn = btn('settings-danger-btn', 'Delete');
  profileActions.appendChild(deleteBtn);
  generalPanel.appendChild(profileActions);
  panelArea.appendChild(generalPanel);

  // ── Audio panel ───────────────────────────────────────────────────────────
  const audioPanel = el('section', 'settings-panel');
  audioPanel.appendChild(sectionLabel('Audio Devices'));

  const audioInputSelect = select('device-select');
  audioInputSelect.appendChild(placeholder('Select device...'));
  const audioOutputSelect = select('device-select');
  audioOutputSelect.appendChild(placeholder('Select device...'));
  audioPanel.append(deviceGroup('Input', audioInputSelect), deviceGroup('Output', audioOutputSelect));
  panelArea.appendChild(audioPanel);

  // ── Radio panel ───────────────────────────────────────────────────────────
  const radioPanel = el('section', 'settings-panel');
  radioPanel.appendChild(sectionLabel('Radio'));

  const radioTypeSelect = select('device-select');
  for (const rt of ['FT-991A']) {
    radioTypeSelect.appendChild(option(rt, rt));
  }
  radioPanel.appendChild(deviceGroup('Radio Type', radioTypeSelect));

  const baudSelect = select('device-select');
  for (const baud of [9600, 19200, 38400, 57600, 115200]) {
    baudSelect.appendChild(option(String(baud), String(baud)));
  }
  radioPanel.appendChild(deviceGroup('Baud Rate', baudSelect));
  panelArea.appendChild(radioPanel);

  // ── Footer ────────────────────────────────────────────────────────────────
  const footer = el('div', 'settings-footer');
  const cancelBtn = btn('settings-cancel-btn', 'Cancel');
  const saveBtn = btn('settings-save-btn', 'Save & Apply');
  footer.append(cancelBtn, saveBtn);
  dialog.appendChild(footer);

  document.body.appendChild(overlay);

  // ── Tab switching ─────────────────────────────────────────────────────────
  const panels: Record<Tab, HTMLElement> = {
    general: generalPanel,
    audio: audioPanel,
    radio: radioPanel,
  };

  function switchTab(t: Tab): void {
    for (const key of TABS) {
      tabBtns[key].classList.toggle('active', key === t);
      panels[key].classList.toggle('active', key === t);
    }
  }
  for (const t of TABS) {
    tabBtns[t].addEventListener('click', () => switchTab(t));
  }

  // ── Open / close ──────────────────────────────────────────────────────────
  function open(tab: Tab = 'general'): void {
    prefillFromConfig(deps.getCurrentConfig());
    switchTab(tab);
    void populateGeneralTab();
    void populateAudioTab();
    requestAnimationFrame(() => overlay.classList.add('settings-visible'));
    profileNameInput.focus();
  }

  function close(): void {
    overlay.classList.remove('settings-visible');
    saveBtn.textContent = 'Save & Apply';
    saveBtn.disabled = false;
  }

  closeBtn.addEventListener('click', close);
  cancelBtn.addEventListener('click', close);
  overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && overlay.classList.contains('settings-visible')) close();
  });

  // ── Dialog-local config snapshot (updated on open and profile switch) ────
  // Tracks the full config being edited so that Save and populateAudioTab
  // don't rely on deps.getCurrentConfig() which may be null or out of date.
  let dialogConfig: Configuration | null = null;

  // ── Pre-fill fields from a config object ─────────────────────────────────
  function prefillFromConfig(config: Configuration | null): void {
    if (!config) return;
    dialogConfig = config;
    profileNameInput.value = config.name;
    audioInputSelect.value = config.audio_input ?? '';
    audioOutputSelect.value = config.audio_output ?? '';
    radioTypeSelect.value = config.radio_type;
    baudSelect.value = String(config.baud_rate);
    deleteBtn.disabled = config.name === 'Default';
  }

  // ── Populate General tab ──────────────────────────────────────────────────
  async function populateGeneralTab(): Promise<void> {
    try {
      const names = await listConfigurations();
      while (profileSelect.options.length > 0) profileSelect.remove(0);
      for (const name of names) {
        profileSelect.appendChild(option(name, name));
      }
      const current = deps.getCurrentConfig();
      if (current && profileSelect.querySelector(`option[value="${current.name}"]`)) {
        profileSelect.value = current.name;
      }
    } catch (err) {
      console.error('Failed to list configurations:', err);
    }
  }

  profileSelect.addEventListener('change', async () => {
    try {
      const config = await loadConfiguration(profileSelect.value);
      prefillFromConfig(config);
    } catch (err) {
      console.error('Failed to load configuration:', err);
    }
  });

  deleteBtn.addEventListener('click', async () => {
    const name = profileSelect.value;
    if (name === 'Default') return;
    if (!confirm(`Delete profile "${name}"?`)) return;
    try {
      await deleteConfiguration(name);
      await populateGeneralTab();
      // Always reset to Default after deletion (populateGeneralTab may leave
      // the select blank if the deleted profile was the active one)
      profileSelect.value = 'Default';
      const config = await loadConfiguration('Default').catch(() => null);
      prefillFromConfig(config);
    } catch (err) {
      console.error('Failed to delete configuration:', err);
    }
  });

  // ── Populate Audio tab ────────────────────────────────────────────────────
  async function populateAudioTab(): Promise<void> {
    let devices: AudioDeviceInfo[] = [];
    try {
      devices = await listAudioDevices();
    } catch (err) {
      console.error('Failed to list audio devices:', err);
      return;
    }

    // Preserve placeholder (index 0), clear the rest
    while (audioInputSelect.options.length > 1) audioInputSelect.remove(1);
    while (audioOutputSelect.options.length > 1) audioOutputSelect.remove(1);

    for (const device of devices) {
      const label = device.name + (device.isDefault ? ' (Default)' : '');
      if (device.isInput) {
        audioInputSelect.appendChild(option(device.id, label));
      } else {
        audioOutputSelect.appendChild(option(device.id, label));
      }
    }

    // Re-apply the in-dialog selection after repopulating (use dialogConfig,
    // not deps.getCurrentConfig(), so a profile switch in the General tab is
    // not silently overwritten by the currently-applied app config)
    const snap = dialogConfig ?? deps.getCurrentConfig();
    if (snap) {
      audioInputSelect.value = snap.audio_input ?? '';
      audioOutputSelect.value = snap.audio_output ?? '';
    }
  }

  // ── Save & Apply ──────────────────────────────────────────────────────────
  saveBtn.addEventListener('click', async () => {
    const name = profileNameInput.value.trim();
    if (!name) {
      profileNameInput.focus();
      return;
    }

    // Use dialogConfig first (set on open and updated on profile switch) so
    // waterfall/serial/carrier fields are never silently reset to defaults
    // if the async startup load hasn't resolved yet.
    const base = dialogConfig ?? deps.getCurrentConfig();
    const config: Configuration = {
      name,
      audio_input: audioInputSelect.value || null,
      audio_output: audioOutputSelect.value || null,
      serial_port: base?.serial_port ?? null,
      baud_rate: parseInt(baudSelect.value, 10),
      radio_type: radioTypeSelect.value,
      carrier_freq: base?.carrier_freq ?? 1000.0,
      waterfall_palette: base?.waterfall_palette ?? 'classic',
      waterfall_noise_floor: base?.waterfall_noise_floor ?? -100,
      waterfall_zoom: base?.waterfall_zoom ?? 1,
    };

    saveBtn.disabled = true;
    saveBtn.textContent = 'Saving…';
    try {
      await deps.onSave(config);
      close();
    } catch (err) {
      console.error('Failed to save settings:', err);
      saveBtn.disabled = false;
      saveBtn.textContent = 'Save & Apply';
    }
  });

  _openDialog = open;
}

// ── DOM helpers ───────────────────────────────────────────────────────────────

function el(tag: string, className: string): HTMLElement {
  const e = document.createElement(tag);
  e.className = className;
  return e;
}

function btn(className: string, text: string): HTMLButtonElement {
  const b = document.createElement('button');
  b.className = className;
  b.textContent = text;
  return b;
}

function select(className: string): HTMLSelectElement {
  const s = document.createElement('select');
  s.className = className;
  return s;
}

function option(value: string, text: string): HTMLOptionElement {
  const o = document.createElement('option');
  o.value = value;
  o.textContent = text;
  return o;
}

function placeholder(text: string): HTMLOptionElement {
  return option('', text);
}

function sectionLabel(text: string): HTMLDivElement {
  const d = document.createElement('div');
  d.className = 'section-label';
  d.textContent = text;
  return d;
}

function deviceGroup(label: string, control: HTMLElement): HTMLDivElement {
  const g = document.createElement('div');
  g.className = 'device-group';
  const lbl = document.createElement('div');
  lbl.className = 'device-label';
  lbl.textContent = label;
  g.append(lbl, control);
  return g;
}
