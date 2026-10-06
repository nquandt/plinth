// plinth:time: clocks and timers (SPEC.md §8.5). No capability is needed.

/** Milliseconds since the Unix epoch, UTC. For display and logging only. */
export declare function now(): number;

/** Monotonic milliseconds since an arbitrary, fixed origin. Use this to
 * measure elapsed time; it never goes backwards. */
export declare function monotonicNow(): number;

/** Calls `callback` once, after `ms` milliseconds. Returns a timer id for
 * `clearTimeout`. */
export declare function setTimeout(callback: () => void, ms: number): number;

/** Calls `callback` every `ms` milliseconds. Returns a timer id for
 * `clearInterval`. */
export declare function setInterval(callback: () => void, ms: number): number;

/** Cancels a timer made with `setTimeout`. Canceling an unknown or
 * already-fired timer is not an error. */
export declare function clearTimeout(id: number): void;

/** Cancels a timer made with `setInterval`. */
export declare function clearInterval(id: number): void;
