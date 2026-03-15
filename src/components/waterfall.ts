/** Waterfall spectrum display - canvas-based scrolling spectrogram */

import { buildAllColorMaps, type ColorPalette } from '../utils/color-map';

export type ZoomLevel = 1 | 2 | 4;

export interface WaterfallSettings {
  palette: ColorPalette;
  noiseFloor: number;
  zoomLevel: ZoomLevel;
}

const AUDIO_START_HZ = 500;
const AUDIO_END_HZ = 2500;
const SAMPLE_RATE = 48000;

export class WaterfallDisplay {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private imageData: ImageData | null = null;  // single-row ImageData for the top line
  private allColorMaps = buildAllColorMaps();
  private colorMap: Uint8ClampedArray[];
  private resizeHandler = () => this.resize();

  // RAF batching: queue incoming rows, drain once per animation frame
  private pendingRows: number[][] = [];
  private rafId: number | null = null;

  // Adjustable settings
  private palette: ColorPalette = 'classic';
  private noiseFloor: number = -100;
  private readonly dynamicRange: number = 80;
  private zoomLevel: ZoomLevel = 1;
  private carrierFreq: number = 1500;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: false })!;
    this.colorMap = this.allColorMaps.classic;
    this.resize();
    window.addEventListener('resize', this.resizeHandler);
  }

  private resize(): void {
    const rect = this.canvas.parentElement!.getBoundingClientRect();
    this.canvas.width = rect.width;
    this.canvas.height = rect.height;
    // Single-row ImageData — we paint one row then use drawImage to scroll
    this.imageData = this.ctx.createImageData(this.canvas.width, 1);
    // Fill canvas black
    this.ctx.fillStyle = '#000';
    this.ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
  }

  start(): void {
    // resize listener is registered in the constructor; nothing extra needed here
  }

  stop(): void {
    window.removeEventListener('resize', this.resizeHandler);
  }

  // --- Settings ---

  setPalette(palette: ColorPalette): void {
    this.palette = palette;
    this.colorMap = this.allColorMaps[palette];
  }

  setNoiseFloor(dbValue: number): void {
    this.noiseFloor = dbValue;
  }

  setZoom(level: ZoomLevel): void {
    this.zoomLevel = level;
  }

  /** Called by click-to-tune so zoom stays centered on the active carrier */
  setCarrierFreq(freqHz: number): void {
    this.carrierFreq = freqHz;
  }

  getSettings(): WaterfallSettings {
    return {
      palette: this.palette,
      noiseFloor: this.noiseFloor,
      zoomLevel: this.zoomLevel,
    };
  }

  /** Returns the currently visible Hz range based on zoom + carrier */
  getVisibleRange(): { startHz: number; endHz: number } {
    if (this.zoomLevel === 1) {
      return { startHz: AUDIO_START_HZ, endHz: AUDIO_END_HZ };
    }
    const span = (AUDIO_END_HZ - AUDIO_START_HZ) / this.zoomLevel; // 1000 or 500
    const half = span / 2;
    const startHz = Math.max(
      AUDIO_START_HZ,
      Math.min(AUDIO_END_HZ - span, this.carrierFreq - half),
    );
    return { startHz, endHz: startHz + span };
  }

  /**
   * Queue a spectrum row for rendering. Schedules a single RAF callback to
   * drain the queue — multiple events arriving in the same frame are collapsed
   * into one paint, preventing main-thread jank from event bursts.
   */
  drawSpectrum(magnitudes: number[]): void {
    this.pendingRows.push(magnitudes);
    if (this.rafId === null) {
      this.rafId = requestAnimationFrame(() => this.flushPendingRows());
    }
  }

  private flushPendingRows(): void {
    this.rafId = null;
    if (!this.imageData || this.pendingRows.length === 0) return;

    const rows = this.pendingRows;
    this.pendingRows = [];

    const { width } = this.canvas;
    const fftSize = rows[0].length * 2;
    const binWidth = SAMPLE_RATE / fftSize;
    const { startHz, endHz } = this.getVisibleRange();
    const startBin = Math.floor(startHz / binWidth);
    const displayBins = Math.ceil(endHz / binWidth) - startBin;
    const data = this.imageData.data;

    for (const magnitudes of rows) {
      // Scroll existing content down by 1px using GPU-accelerated drawImage
      this.ctx.drawImage(this.canvas, 0, 1);

      // Paint the new row into the single-row ImageData
      for (let x = 0; x < width; x++) {
        const binIdx = Math.floor(startBin + (x / width) * displayBins);
        const db = binIdx < magnitudes.length ? magnitudes[binIdx] : this.noiseFloor;
        const normalized = Math.min(
          255,
          Math.max(0, Math.floor(((db - this.noiseFloor) / this.dynamicRange) * 255)),
        );
        const color = this.colorMap[normalized];
        const idx = x * 4;
        data[idx]     = color[0];
        data[idx + 1] = color[1];
        data[idx + 2] = color[2];
        data[idx + 3] = 255;
      }

      // Write only the top row (dirty-rect putImageData)
      this.ctx.putImageData(this.imageData, 0, 0);
    }
  }

}
