export interface Rect { x: number; y: number; width: number; height: number }
export const outward = (x1: number, y1: number, x2: number, y2: number): Rect => {
  const x = Math.floor(Math.min(x1, x2)), y = Math.floor(Math.min(y1, y2));
  return { x, y, width: Math.ceil(Math.max(x1, x2)) - x, height: Math.ceil(Math.max(y1, y2)) - y };
};
