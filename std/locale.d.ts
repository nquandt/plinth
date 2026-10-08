// plinth:locale: the locale of the app, and formatting in it (core 1.13,
// docs/I18N.md §3.1, §3.7, §4 steps 1 and 2; docs/host-apis.md
// "plinth:locale"). Needs no capability, as for the time zone.
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

// -- Formatting (core 1.13, docs/I18N.md §3.7, §4 step 2) ----------------------
// The host formats in the locale of the app: ICU4X on the desktop, the
// browser's `Intl` on the web. Each function is reactive, as `locale()`: a
// text that calls it shows the new format when the locale changes. Write the
// options as an object literal at the call. A host can give a slightly
// different text from another host in some cases; docs/I18N.md §3.7 lists
// the known differences.

/** The options of `formatNumber`. Each field is optional. */
export interface NumberFormatOptions {
  /** "decimal" (the default), "percent" (0.25 is "25%") or "currency". */
  style?: "decimal" | "percent" | "currency";
  /** The ISO 4217 code for the "currency" style, for example "EUR". Without
   * a valid code, the number shows in the "decimal" style. */
  currency?: string;
  /** The minimum number of digits after the decimal separator (0 to 20).
   * Default: 0. Not used for "currency" and "compact" (the currency or the
   * locale sets the digits). */
  minimumFractionDigits?: number;
  /** The maximum number of digits after the decimal separator (0 to 20).
   * Default: 3 for "decimal", 0 for "percent". Not used for "currency" and
   * "compact". */
  maximumFractionDigits?: number;
  /** Group the digits ("1,234,567"). Default: true (the locale decides, for
   * example Polish does not group a number of 4 digits). Not used for
   * "currency" and "compact". */
  useGrouping?: boolean;
  /** "standard" (the default) or "compact" ("1.2K"). "compact" applies to
   * the "decimal" style only. */
  notation?: "standard" | "compact";
}

/** The options of `formatDate`. Each field is optional. */
export interface DateFormatOptions {
  /** The date: "full" (with the weekday), "long", "medium" or "short". */
  dateStyle?: "full" | "long" | "medium" | "short";
  /** The time: "medium" (with seconds) or "short". */
  timeStyle?: "medium" | "short";
}

/** A CLDR plural category. */
export type PluralCategory = "zero" | "one" | "two" | "few" | "many" | "other";

/** "cardinal" (1 file, 2 files) or "ordinal" (1st, 2nd). */
export type PluralType = "cardinal" | "ordinal";

/** "conjunction" ("a, b, and c") or "disjunction" ("a, b, or c"). */
export type ListType = "conjunction" | "disjunction";

/** `value` as text in the locale of the app, for example "1,234.5" in
 * en-US and "1.234,5" in de-DE. A value that is not finite gives "".
 * Reactive. */
export declare function formatNumber(value: number, options?: NumberFormatOptions): string;

/** The time `ms` (milliseconds since the Unix epoch, as `now()`) as text in
 * the locale of the app and in the local time zone. Without options, as
 * `{ dateStyle: "medium" }`. A value that is not finite gives "". Reactive.
 * (`formatDate` of `plinth:time` formats with a pattern and English names.) */
export declare function formatDate(ms: number, options?: DateFormatOptions): string;

/** The plural category of `value` in the locale of the app, for example
 * "one" for 1 and "other" for 2 in English, "few" for 3 in Polish.
 * Reactive. */
export declare function pluralCategory(value: number, type?: PluralType): PluralCategory;

/** The strings as one list in the locale of the app, for example
 * "a, b, and c". Reactive. */
export declare function formatList(items: string[], type?: ListType): string;

/** Compares two strings in the order of the locale (for sorting names): -1
 * when `a` comes first, 0 when they are equal, 1 when `b` comes first.
 * Reactive. */
export declare function compare(a: string, b: string): number;
