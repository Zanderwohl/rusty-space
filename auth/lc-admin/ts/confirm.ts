/**
 * A confirmation that looks like the rest of the console, in place of `window.confirm` — which
 * is browser chrome with the origin in it and no room for the name of the account being banned.
 *
 * **Exactly one of `issueRequest`/`dropRequest` must be called**, or the request hangs for the
 * life of the page — hence resolving on `close`, however the dialog was closed.
 */

/** Reused: a dialog per request leaks one element per click. */
let dialog: HTMLDialogElement | null = null;

function ensureDialog(): HTMLDialogElement {
  if (dialog) return dialog;
  const made = document.createElement("dialog");
  made.className = "confirm";
  made.innerHTML = `
    <form method="dialog">
      <p class="confirm-said"></p>
      <div class="confirm-buttons">
        <button value="no">Cancel</button>
        <button value="yes" class="danger">Confirm</button>
      </div>
    </form>`;
  document.body.append(made);
  dialog = made;
  return made;
}


function ask(said: string, confirmLabel: string): Promise<boolean> {
  const made = ensureDialog();
  const text = made.querySelector<HTMLParagraphElement>(".confirm-said");
  if (text) text.textContent = said;
  const yes = made.querySelector<HTMLButtonElement>('button[value="yes"]');
  if (yes) yes.textContent = confirmLabel;

  return new Promise((resolve) => {
    const settle = () => {
      made.removeEventListener("close", settle);
      // Empty for Escape and for a backdrop dismissal, both of which are "no".
      resolve(made.returnValue === "yes");
    };
    made.addEventListener("close", settle);
    made.returnValue = "";
    made.showModal();
    // The safe choice, or a held-down Return answers for you.
    made.querySelector<HTMLButtonElement>('button[value="no"]')?.focus();
  });
}

export function install(): void {
  document.body.addEventListener("htmx:confirm", (event) => {
    const detail = event.detail;
    const said = detail.ctx.sourceElement?.getAttribute("hx-confirm");
    if (!said) return;

    event.preventDefault();

    // So "Ban" is confirmed with "Ban" rather than "OK".
    const button = detail.ctx.sourceElement?.querySelector<HTMLButtonElement>(
      'button[type="submit"]',
    );
    const label = button?.textContent?.trim() || "Confirm";

    void ask(said, label).then((yes) => {
      if (yes) detail.issueRequest();
      else detail.dropRequest();
    });
  });
}
