/** Shared type definitions matching Rust domain types */

export interface AudioDeviceInfo {
  id: string;
  name: string;
  isInput: boolean;
  isDefault: boolean;
}

export interface SerialPortInfo {
  name: string;
  portType: string;
}

export interface Configuration {
  name: string;
  audio_input: string | null;
  audio_output: string | null;
  serial_port: string | null;
  baud_rate: number;
  radio_type: string;
  carrier_freq: number;
  waterfall_palette: string;
  waterfall_noise_floor: number;
  waterfall_zoom: number;
}

export interface RadioInfo {
  port: string;
  baudRate: number;
  frequencyHz: number;
  mode: string;
  connected: boolean;
}

export interface MenuEvent {
  id: string;
}

export interface ConnectionStatus {
  serialConnected: boolean;
  serialPort: string | null;
  audioStreaming: boolean;
  audioDevice: string | null;
}
