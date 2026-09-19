/**
 * A confirmation that looks like the rest of the console.
 *
 * `hx-confirm` opens `window.confirm` by default, which is a browser chrome dialog with the
 * origin in it and no room for the name of the account being banned. htmx 4 hands out an
 * asynchronous escape hatch — `htmx:confirm` carries `issueRequest` and `dropRequest` — so
 * this replaces the dialog and nothing else about the mechanism.
 *
 * **Exactly one of the two callbacks must be called.** Neither leaves the request pending for
 * the life of the page, which is why the dialog's close handler resolves it whatever the
 * closing was: the Escape key, the backdrop, or a button.
 */

/** The dialog, made once and reused. A dialog per request leaks one element per click. */
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

/** Ask, and answer with what was chosen. */
function ask(said: string, confirmLabel: string): Promise<boolean> {
  const made = ensureDialog();
  const text = made.querySelector<HTMLParagraphElement>(".confirm-said");
  if (text) text.textContent = said;
  const yes = made.querySelector<HTMLButtonElement>('button[value="yes"]');
  if (yes) yes.textContent = confirmLabel;

  return new Promise((resolve) => {
    const settle = () => {
      made.removeEventListener("close", settle);
      // `returnValue` is empty for Escape and for a backdrop dismissal, which are both "no".
      resolve(made.returnValue === "yes");
    };
    made.addEventListener("close", settle);
    made.returnValue = "";
    made.showModal();
    // Focus the safe choice. A dialog that opens with the destructive button focused is one
    // that a held-down Return key answers for you.
    made.querySelector<HTMLButtonElement>('button[value="no"]')?.focus();
  });
}

export function install(): void {
  document.body.addEventListener("htmx:confirm", (event) => {
    const detail = event.detail;
    const said = detail.ctx.sourceElement?.getAttribute("hx-confirm");
    if (!said) return;
    // Taking the question means taking responsibility for answering it.
    event.preventDefault();

    // The submit button's own words, so "Ban" is confirmed with "Ban" rather than with "OK".
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
