/**
 * Whether the crews' column is out, decided from where the pointer is.
 *
 * The column stood open and cost every window four rem of its width to offer a
 * choice most operators make a few times a day. Collapsed to a band at the edge
 * it costs half a rem, and the way back to it is moving toward it, which is the
 * gesture somebody reaching for the left edge of the window is already making.
 *
 * Two thresholds rather than one, and the gap between them is the whole point.
 * A single line means the column closes at exactly the pixel that opens it, so
 * a hand resting at the boundary flickers it, and the moment it starts to close
 * the pointer is inside the zone again. So: coming within `reach.right` opens
 * it, going past the far side of the column plus that same distance again
 * closes it, and in between it is left as it was.
 *
 * The numbers are read off the DOM rather than written here, because both of
 * them are lengths and every length in this app is named in one stylesheet at
 * one scale. `reach` is a box CSS positions and sizes and nothing can click,
 * and its `top` is the second half of that: on macOS the window's own buttons
 * float over the top left corner, and a zone that ran to the top of the window
 * would slide the crews out every time somebody went to close the app.
 *
 * Read from the pointer rather than from `:hover` on a strip over the rail.
 * A strip wide enough to be aimed at is a strip that swallows clicks on the
 * left edge of every agent row behind it, and a row that does nothing when it
 * is clicked near its left side is a worse bug than the one this fixes.
 */

export interface Reach {
  /**
   * The proximity zone: come inside it and the column comes out. Its `top` is
   * where the zone starts, which is under the window's own buttons.
   */
  reach: { top: number; right: number };
  /** The column itself, wherever it currently is. */
  column: { right: number };
}

/** Where the pointer is, in window coordinates. */
export interface At {
  x: number;
  y: number;
}

export function reaches(open: boolean, at: At, boxes: Reach): boolean {
  const { reach, column } = boxes;
  if (at.x <= reach.right && at.y >= reach.top) return true;
  if (at.x > column.right + reach.right) return false;
  return open;
}
