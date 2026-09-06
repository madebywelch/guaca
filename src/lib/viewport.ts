/** Keep the composer above a mobile keyboard, including Safari's viewport pan.
 * CSS consumes these measurements only in the narrow layout. Pinch zoom keeps
 * the layout's original dimensions so magnifying text does not reflow it.
 */
export function followViewport(): () => void {
  const viewport = window.visualViewport;
  if (!viewport) return () => {};
  const style = document.documentElement.style;
  const update = () => {
    if (viewport.scale !== 1) return;
    style.setProperty("--viewport-height", `${viewport.height}px`);
    style.setProperty("--viewport-top", `${viewport.offsetTop}px`);
  };
  update();
  viewport.addEventListener("resize", update);
  viewport.addEventListener("scroll", update);
  return () => {
    viewport.removeEventListener("resize", update);
    viewport.removeEventListener("scroll", update);
    style.removeProperty("--viewport-height");
    style.removeProperty("--viewport-top");
  };
}
