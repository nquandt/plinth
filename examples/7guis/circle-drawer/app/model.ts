// The model: the circles, the selection, and undo and redo. The view
// (main.tsx) reads it and calls these functions. Positions and diameters
// are view units of the canvas (VIEW_W x VIEW_H).
import { signal, computed } from "plinth:ui";

export const VIEW_W = 400;
export const VIEW_H = 300;
export const NEW_DIAMETER = 30;
export const MIN_DIAMETER = 4;
export const MAX_DIAMETER = 150;

export interface Circle {
  id: number;
  x: number;
  y: number;
  d: number;
}

let nextId = 1;

export const circles = signal<Circle[]>([]);
/** Earlier and later states of `circles`, for undo and redo. */
const undoStack = signal<Circle[][]>([]);
const redoStack = signal<Circle[][]>([]);
export const canUndo = computed(() => undoStack().length > 0);
export const canRedo = computed(() => redoStack().length > 0);

/** The id of the selected circle, or 0 for no selection. */
export const selectedId = signal(0);
export const selected = computed(() => circles().find((c) => c.id === selectedId()) ?? null);

/** Saves the current state for undo; a new change clears redo. */
function remember(): void {
  undoStack.set([...undoStack(), circles()]);
  redoStack.set([]);
}

/** The circle nearest to (x, y) that contains it, or null. */
export function circleAt(x: number, y: number): Circle | null {
  let best: Circle | null = null;
  let bestDist = 0;
  for (const c of circles()) {
    const dx = c.x - x;
    const dy = c.y - y;
    const dist = dx * dx + dy * dy;
    if (dist <= (c.d / 2) * (c.d / 2) && (best === null || dist < bestDist)) {
      best = c;
      bestDist = dist;
    }
  }
  return best;
}

/** A click: selects the circle under the pointer, or makes a new one there. */
export function click(x: number, y: number): void {
  const hit = circleAt(x, y);
  if (hit !== null) {
    selectedId.set(hit.id);
    return;
  }
  remember();
  const c: Circle = { id: nextId, x: x, y: y, d: NEW_DIAMETER };
  nextId = nextId + 1;
  circles.set([...circles(), c]);
  selectedId.set(c.id);
}

/** Selects the circle under the pointer (a hover). */
export function hover(x: number, y: number): void {
  const hit = circleAt(x, y);
  selectedId.set(hit === null ? 0 : hit.id);
}

/** The diameter of the selected circle before the adjust sheet opened. */
let adjustFrom = 0;

export function beginAdjust(): void {
  const s = selected();
  adjustFrom = s === null ? 0 : s.d;
}

/** Changes the selected circle's diameter at once (no undo step yet). */
export function setDiameter(d: number): void {
  const id = selectedId();
  circles.set(circles().map((c) => (c.id === id ? { id: c.id, x: c.x, y: c.y, d: d } : c)));
}

/** Closes the adjust sheet: one undo step for the whole change. */
export function endAdjust(): void {
  const s = selected();
  if (s === null || s.d === adjustFrom) {
    return;
  }
  const now = circles();
  // The undo state is the circles with the old diameter.
  circles.set(now.map((c) => (c.id === s.id ? { id: c.id, x: c.x, y: c.y, d: adjustFrom } : c)));
  remember();
  circles.set(now);
}

export function undo(): void {
  const stack = undoStack();
  if (stack.length === 0) {
    return;
  }
  redoStack.set([...redoStack(), circles()]);
  circles.set(stack[stack.length - 1]);
  undoStack.set(stack.slice(0, -1));
}

export function redo(): void {
  const stack = redoStack();
  if (stack.length === 0) {
    return;
  }
  undoStack.set([...undoStack(), circles()]);
  circles.set(stack[stack.length - 1]);
  redoStack.set(stack.slice(0, -1));
}
