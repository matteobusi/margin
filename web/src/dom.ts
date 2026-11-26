// Tiny DOM helper. Text is always set with textContent (no innerHTML with user data).
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  for (const c of children) el.append(typeof c === "string" ? document.createTextNode(c) : c);
  return el;
}

/** A drop zone that also opens a file picker on click. */
export function dropZone(
  label: string,
  opts: { directory?: boolean; accept?: string; multiple?: boolean },
  onFiles: (files: File[]) => void,
): HTMLElement {
  const input = h("input", { type: "file", hidden: "" });
  if (opts.accept) input.accept = opts.accept;
  if (opts.multiple) input.multiple = true;
  if (opts.directory) {
    input.setAttribute("webkitdirectory", "");
    input.multiple = true;
  }
  const zone = h("div", { class: "drop", tabindex: "0", role: "button" }, label, input);
  zone.addEventListener("click", () => input.click());
  zone.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") input.click();
  });
  input.addEventListener("change", () => onFiles([...(input.files ?? [])]));
  zone.addEventListener("dragover", (e) => {
    e.preventDefault();
    zone.classList.add("over");
  });
  zone.addEventListener("dragleave", () => zone.classList.remove("over"));
  zone.addEventListener("drop", (e) => {
    e.preventDefault();
    zone.classList.remove("over");
    onFiles([...(e.dataTransfer?.files ?? [])]);
  });
  return zone;
}
