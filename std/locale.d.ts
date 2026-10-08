// plinth:locale: the locale of the app (core 1.13, docs/I18N.md §3.1, §4
// step 1; docs/host-apis.md "plinth:locale"). Needs no capability, as for the
// time zone.
//
// The host gives the locale: the OS setting on the desktop, the browser
// language on the web. Both functions are reactive, as `isActive()` in
// `plinth:ui`: a text, a computed value or an effect that reads them runs
// again when the user changes the locale while the app runs.

/** The text direction of a locale. */
export type Direction = "ltr" | "rtl";

/** The locale of the app as a BCP 47 tag, for example "en-US", "ar-EG" or
 * "zh-Hans-CN". Never empty. Reactive. */
export declare function locale(): string;

/** "rtl" when the script of the locale is written right to left (for
 * example Arabic, Hebrew, Persian, Urdu), else "ltr". Reactive. */
export declare function direction(): Direction;
