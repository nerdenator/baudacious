/** Startup recovery dialog — shown when auto-connect fails or no port is configured */

export interface StartupRecoveryOptions {
  onRetry: () => void;
  onSelectProfile: () => void;
  onReconfigure: () => void;
  onExit: () => void;
}

let overlay: HTMLElement | null = null;

export function showStartupRecoveryDialog(
  profileName: string,
  error: string,
  options: StartupRecoveryOptions,
): void {
  if (overlay) overlay.remove();

  overlay = document.createElement('div');
  overlay.className = 'startup-overlay';

  const dialog = document.createElement('div');
  dialog.className = 'startup-dialog';

  const title = document.createElement('h2');
  title.className = 'startup-title';
  title.textContent = 'Radio Not Connected';

  const body = document.createElement('p');
  body.className = 'startup-body';
  body.textContent = `Could not connect using profile "${profileName}": ${error}`;

  const btnRetry = document.createElement('button');
  btnRetry.className = 'startup-recovery-btn startup-retry-btn';
  btnRetry.textContent = 'Retry connection';
  btnRetry.addEventListener('click', () => {
    btnRetry.disabled = true;
    btnRetry.textContent = 'Connecting…';
    options.onRetry();
  });

  const btn1 = document.createElement('button');
  btn1.className = 'startup-recovery-btn';
  btn1.textContent = 'Select a different profile';
  btn1.addEventListener('click', () => { hideStartupRecoveryDialog(); options.onSelectProfile(); });

  const btn2 = document.createElement('button');
  btn2.className = 'startup-recovery-btn';
  btn2.textContent = 'Reconfigure this profile';
  btn2.addEventListener('click', () => { hideStartupRecoveryDialog(); options.onReconfigure(); });

  const btn3 = document.createElement('button');
  btn3.className = 'startup-recovery-btn startup-exit-btn';
  btn3.textContent = 'Exit';
  btn3.addEventListener('click', () => options.onExit());

  dialog.append(title, body, btnRetry, btn1, btn2, btn3);
  overlay.appendChild(dialog);
  document.body.appendChild(overlay);
}

export function hideStartupRecoveryDialog(): void {
  overlay?.remove();
  overlay = null;
}
