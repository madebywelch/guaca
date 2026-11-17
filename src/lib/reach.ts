export interface Reach {
  reach: { top: number; right: number };
  column: { right: number };
}

export interface At {
  x: number;
  y: number;
}

export function reaches(open: boolean, at: At, boxes: Reach, holding: boolean): boolean {
  const { reach, column } = boxes;
  if (at.x <= reach.right && at.y >= reach.top) return true;
  if (holding) return open;
  if (at.x > column.right + reach.right) return false;
  return open;
}
