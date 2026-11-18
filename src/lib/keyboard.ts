export function watchKeyboard(target: Window): () => void {
  const root = target.document.documentElement;
  const keyed = () => {
    root.dataset.keyboard = "";
  };
  const pointed = () => {
    delete root.dataset.keyboard;
  };
  target.addEventListener("keydown", keyed, true);
  target.addEventListener("pointerdown", pointed, true);
  target.addEventListener("focus", pointed);
  target.addEventListener("blur", pointed);
  return () => {
    target.removeEventListener("keydown", keyed, true);
    target.removeEventListener("pointerdown", pointed, true);
    target.removeEventListener("focus", pointed);
    target.removeEventListener("blur", pointed);
    pointed();
  };
}
