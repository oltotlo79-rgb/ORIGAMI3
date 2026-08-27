function openDialog(dialog: HTMLDialogElement): void {
  if (typeof dialog.showModal === "function") {
    dialog.showModal();
    return;
  }
  dialog.setAttribute("open", "");
}

function closeDialog(dialog: HTMLDialogElement): void {
  if (typeof dialog.close === "function") {
    dialog.close();
    return;
  }
  dialog.removeAttribute("open");
}

export function setupWebShell(page: Document): void {
  const openButton = page.querySelector<HTMLButtonElement>("#manual-download");
  const dialog = page.querySelector<HTMLDialogElement>("#manual-confirmation");
  const yesLink = page.querySelector<HTMLAnchorElement>("#manual-confirm-yes");
  const noButton = page.querySelector<HTMLButtonElement>("#manual-confirm-no");

  if (!openButton || !dialog || !yesLink || !noButton) return;

  openButton.addEventListener("click", () => openDialog(dialog));
  yesLink.addEventListener("click", () => closeDialog(dialog));
  noButton.addEventListener("click", () => closeDialog(dialog));
}
