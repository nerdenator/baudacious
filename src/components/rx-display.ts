/** RX (receive) display panel */

let rxContentEl: HTMLElement | null = null;

/** Append decoded text to the RX display and auto-scroll to bottom */
export function appendRxText(text: string): void {
  if (!rxContentEl) return;
  rxContentEl.appendChild(document.createTextNode(text));

  // Auto-scroll: keep the view pinned to the bottom as new text arrives
  rxContentEl.scrollTop = rxContentEl.scrollHeight;
}

export function setupRxDisplay(): void {
  const clearBtn = document.querySelector('.rx-controls .rx-btn') as HTMLButtonElement;
  rxContentEl = document.getElementById('rx-content') as HTMLElement;

  if (clearBtn && rxContentEl) {
    clearBtn.addEventListener('click', () => {
      rxContentEl!.textContent = '';
    });
  }
}
