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

/** Calls `callback` before each frame that the screen draws (core 1.12),
 * with `dt`, the milliseconds since the previous frame (0 for the first).
 * Use it for motion: move by `speed * dt`, not by a fixed step. The host
 * sends no frames while the window is hidden, so `dt` can be large after
 * the window comes back; limit it. Returns an id for `cancelFrame`. */
export declare function onFrame(callback: (dt: number) => void): number;

/** Stops a frame timer made with `onFrame`. */
export declare function cancelFrame(id: number): void;

// -- Date and time (SPEC.md §4.7, §8.5; docs/GAPS.md gap #5) ---------------
//
// `now()` already gives milliseconds since the Unix epoch, UTC. These add
// the local time zone, calendar math, formatting and parsing. Formatting
// is English-only for now; locale-aware formatting is `plinth:locale`
// (SPEC.md §4.7), later.

/** The local time zone's offset from UTC, in minutes east of UTC, at the
 * instant `ms` (milliseconds since the Unix epoch). Negative west of UTC. */
export declare function timezoneOffset(ms: number): number;

/** A timestamp broken into calendar fields. `month` is 1-12; `weekday` is
 * 0 (Sunday) to 6 (Saturday). */
export interface DateParts {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
  millisecond: number;
  weekday: number;
}

/** Breaks `ms` into calendar fields, in the local time zone unless `utc`
 * is true. */
export declare function dateParts(ms: number, utc?: boolean): DateParts;

/** Builds a timestamp from local wall-clock calendar fields (the inverse
 * of `dateParts`); `hour`, `minute` and `second` default to 0. */
export declare function makeDate(year: number, month: number, day: number, hour?: number, minute?: number, second?: number): number;

/** Formats `ms` with a small pattern language, in the local time zone
 * unless `utc` is true. Tokens: `YYYY` `MM` `M` `DD` `D` `HH` `H` `hh` `h`
 * `mm` `ss` `SSS` `ddd` `dddd` `MMM` `MMMM` `A`; any other character is
 * copied through unchanged. Names are English. */
export declare function formatDate(ms: number, pattern: string, utc?: boolean): string;

/** `ms` as an ISO-8601 string in UTC, like JS's `Date.prototype.
 * toISOString` (e.g. `"2024-03-05T09:07:03.045Z"`). */
export declare function toISOString(ms: number): string;

/** Parses `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS[.sss]][Z|±HH:MM]`. With no
 * `Z`/offset, `text` is local time. Returns `null` if `text` does not
 * match either form. Consistent with what `DatePicker` stores. */
export declare function parseDate(text: string): number | null;
