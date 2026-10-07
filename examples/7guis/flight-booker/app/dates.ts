// Dates as the 7GUIs task writes them: "DD.MM.YYYY".
import { makeDate, dateParts } from "plinth:time";
import { parseNumber } from "plinth:core";

/** A calendar day, as a number that sorts like the date: YYYYMMDD. */
export type Day = number;

function digits(s: string, length: number): boolean {
  if (s.length !== length) {
    return false;
  }
  for (let i = 0; i < s.length; i++) {
    const c = s.charAt(i);
    if (c < "0" || c > "9") {
      return false;
    }
  }
  return true;
}

/** The day in `text` ("DD.MM.YYYY"), or null if it is not a real date. */
export function parseDay(text: string): Day | null {
  const parts = text.trim().split(".");
  if (parts.length !== 3 || !digits(parts[0], 2) || !digits(parts[1], 2) || !digits(parts[2], 4)) {
    return null;
  }
  const day = parseNumber(parts[0]);
  const month = parseNumber(parts[1]);
  const year = parseNumber(parts[2]);
  if (month < 1 || month > 12 || day < 1 || day > 31) {
    return null;
  }
  // `makeDate` rolls an impossible day over into the next month (31.04
  // becomes 01.05). Read the fields back to find such a day.
  const p = dateParts(makeDate(year, month, day, 12));
  if (p.year !== year || p.month !== month || p.day !== day) {
    return null;
  }
  return year * 10000 + month * 100 + day;
}
