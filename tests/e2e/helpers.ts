/**
 * Shared Playwright E2E test helpers.
 *
 * mockInvoke extracted from serial.spec.ts, audio.spec.ts, and rx.spec.ts
 * where it was previously copy-pasted into each file.
 *
 * The Tauri 2.x event system uses transformCallback + plugin:event|listen rather
 * than window.dispatchEvent('tauri://event'). This file provides a complete mock
 * of that mechanism so listen()-based event handlers can be triggered in tests.
 */

import { type Page } from '@playwright/test';

/**
 * Mock window.__TAURI_INTERNALS__.invoke and set up a complete Tauri 2.x event
 * infrastructure so that listen()-based event handlers can be triggered in tests
 * via fireEvent().
 *
 * The Tauri SDK's listen() calls:
 *   1. transformCallback(handler)  → stores handler, returns numeric ID
 *   2. invoke('plugin:event|listen', { event, handler: ID })  → registers the mapping
 *
 * When backend fires an event, it calls __TAURI_INTERNALS__.callbacks[ID](eventObj).
 * We reproduce this by exposing window.__dispatchTauriEvent__(event, payload) which
 * calls the stored handlers directly.
 */
export function mockInvoke(
  page: Page,
  handlers: Record<string, unknown>,
) {
  return page.addInitScript((h) => {
    // ── Tauri 2.x event infrastructure ──────────────────────────────────────

    // callbacks[id] = wrapped handler function (set by transformCallback)
    const _callbacks: Record<number, (eventObj: unknown) => void> = {};
    let _nextId = 1;

    // _eventHandlers[eventName] = list of dispatcher functions
    // Each dispatcher calls the appropriate _callbacks entry with a full event object
    const _eventHandlers: Record<string, Array<(payload: unknown) => void>> = {};

    // ── __TAURI_INTERNALS__ ─────────────────────────────────────────────────
    (window as any).__TAURI_INTERNALS__ = {
      callbacks: _callbacks,

      /**
       * Called by the Tauri SDK's listen() to register a handler function and
       * get back a numeric ID that is then passed to plugin:event|listen.
       */
      transformCallback(callback?: (eventObj: unknown) => void, once = false): number {
        const id = _nextId++;
        _callbacks[id] = once
          ? (eventObj) => { delete _callbacks[id]; callback?.(eventObj); }
          : (eventObj) => { callback?.(eventObj); };
        return id;
      },

      invoke(cmd: string, args?: any) {
        // Handle Tauri event plugin commands ─────────────────────────────────
        if (cmd === 'plugin:event|listen') {
          const eventName: string = args?.event ?? '';
          const handlerId: number = args?.handler ?? 0;
          if (eventName && handlerId) {
            (_eventHandlers[eventName] ??= []).push((payload) => {
              _callbacks[handlerId]?.({
                event: eventName,
                payload,
                id: handlerId,
                windowLabel: 'main',
              });
            });
          }
          return Promise.resolve(handlerId);
        }
        if (cmd === 'plugin:event|unlisten') {
          return Promise.resolve(null);
        }
        if (cmd === 'plugin:event|emit') {
          return Promise.resolve(null);
        }

        // Handle user-provided handlers ────────────────────────────────────
        if (cmd in h) {
          const value = h[cmd];
          if (typeof value === 'function') {
            return Promise.resolve((value as any)(args));
          }
          return Promise.resolve(value);
        }

        // Default: return null for unmocked commands
        return Promise.resolve(null);
      },

      metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
      convertFileSrc: (src: string) => src,
    };

    // ── Test-only event dispatcher ──────────────────────────────────────────
    /**
     * Dispatch a simulated backend event to all registered listen() handlers.
     * Used by the fireEvent() helper in tests.
     */
    (window as any).__dispatchTauriEvent__ = (eventName: string, payload: unknown) => {
      const dispatchers = _eventHandlers[eventName] ?? [];
      dispatchers.forEach((dispatch) => dispatch(payload));
    };
  }, handlers);
}

/**
 * Dispatch a Tauri backend event into the frontend's listen() handlers.
 *
 * Requires that mockInvoke() has been called first (it sets up __dispatchTauriEvent__).
 * This properly routes the event through the registered listen() handlers, unlike
 * the old window.dispatchEvent('tauri://event') approach which only worked in Tauri 1.x.
 */
export async function fireEvent(
  page: Page,
  event: string,
  payload: unknown,
): Promise<void> {
  await page.evaluate(
    ([e, p]) => (window as any).__dispatchTauriEvent__(e, p),
    [event, payload] as const,
  );
}

/**
 * Dismiss the startup recovery dialog if it is present.
 *
 * The startup recovery dialog is shown on page load when no serial port is
 * configured (or auto-connect fails). Tests that interact with UI elements
 * behind the dialog must call this after page.goto('/').
 */
export async function dismissStartupDialog(page: Page): Promise<void> {
  await page.evaluate(() => {
    const overlay = document.querySelector('.startup-overlay');
    if (overlay) overlay.remove();
  });
}
