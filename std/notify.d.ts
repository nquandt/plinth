// plinth:notify: system notifications (core 1.12, docs/PRODUCTION.md H3;
// docs/host-apis.md "plinth:notify"). Needs the `notify` capability (Medium
// risk: the user is asked one time). The host shows the notification in the
// system notification area, outside the app window.
//
// `show` returns at once with a `Promise`. It resolves with "shown" when the
// host showed the notification, or with "unsupported" when this host has no
// notification area (the app can then show the message in its own window).
// It rejects with an `Error` when the call is denied; the message is
// "denied:undeclared", "denied:refused" or "denied:unsupported". A denied
// call never traps the app.

/** The options of `show`. Write them as an object literal at the call. */
export interface NotifyOptions {
  /** The first line of the notification. */
  title: string;
  /** More text below the title. */
  body?: string;
  /** A name for the notification: a new notification with the same tag
   * replaces the old one, and `cancel(tag)` removes it. */
  tag?: string;
}

/** What `show` resolves with. */
export type NotifyResult = "shown" | "unsupported";

/** Shows a system notification. */
export declare function show(options: NotifyOptions): Promise<NotifyResult>;

/** Removes the notification with `tag`, if the host still shows it. A tag
 * that the host does not show is not an error. A denied call does nothing;
 * `lastError` then gives the reason. */
export declare function cancel(tag: string): void;

/** The reason the last `cancel` was denied: one of `"denied:undeclared"`,
 * `"denied:refused"`, `"denied:unsupported"`, or `null` if it was not
 * denied. A denied `show` rejects its promise instead. */
export declare function lastError(): string | null;
