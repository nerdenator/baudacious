/** Typed wrappers for all Tauri backend commands */

import { invoke } from '@tauri-apps/api/core';
import type { Configuration, AudioDeviceInfo, SerialPortInfo, RadioInfo, RadioStatus, ConnectionStatus } from '../types';

// Audio commands
export async function listAudioDevices(): Promise<AudioDeviceInfo[]> {
  return invoke('list_audio_devices');
}

export async function startAudioStream(deviceId: string): Promise<void> {
  return invoke('start_audio_stream', { deviceId });
}

export async function stopAudioStream(): Promise<void> {
  return invoke('stop_audio_stream');
}

// RX commands
export async function startRx(): Promise<void> {
  return invoke('start_rx');
}

export async function stopRx(): Promise<void> {
  return invoke('stop_rx');
}

export async function setCarrierFrequency(freqHz: number): Promise<void> {
  return invoke('set_carrier_frequency', { freqHz });
}

// TX commands
export async function startTx(text: string, deviceId: string): Promise<void> {
  return invoke('start_tx', { text, deviceId });
}

export async function stopTx(): Promise<void> {
  return invoke('stop_tx');
}

export async function startTune(deviceId: string): Promise<void> {
  return invoke('start_tune', { deviceId });
}

export async function stopTune(): Promise<void> {
  return invoke('stop_tune');
}

// Serial commands
export async function listSerialPorts(): Promise<SerialPortInfo[]> {
  return invoke('list_serial_ports');
}

export async function connectSerial(port: string, baudRate: number): Promise<RadioInfo> {
  return invoke('connect_serial', { port, baudRate });
}

export async function disconnectSerial(): Promise<void> {
  return invoke('disconnect_serial');
}

// Radio commands
export async function pttOn(): Promise<void> {
  return invoke('ptt_on');
}

export async function pttOff(): Promise<void> {
  return invoke('ptt_off');
}

export async function getFrequency(): Promise<number> {
  return invoke('get_frequency');
}

export async function setFrequency(freqHz: number): Promise<void> {
  return invoke('set_frequency', { freqHz });
}

export async function getMode(): Promise<string> {
  return invoke('get_mode');
}

export async function setMode(mode: string): Promise<void> {
  return invoke('set_mode', { mode });
}

// Configuration commands
export async function saveConfiguration(config: Configuration): Promise<void> {
  return invoke('save_configuration', { config });
}

export async function loadConfiguration(name: string): Promise<Configuration> {
  return invoke('load_configuration', { name });
}

export async function listConfigurations(): Promise<string[]> {
  return invoke('list_configurations');
}

export async function deleteConfiguration(name: string): Promise<void> {
  return invoke('delete_configuration', { name });
}

// Status commands
export async function getConnectionStatus(): Promise<ConnectionStatus> {
  return invoke('get_connection_status');
}

export async function getRadioState(): Promise<RadioStatus> {
  return invoke('get_radio_state');
}

export async function getTxPower(): Promise<number> {
  return invoke<number>('get_tx_power');
}

export async function setTxPowerConfig(watts: number): Promise<void> {
  return invoke('set_tx_power_config', { watts });
}
