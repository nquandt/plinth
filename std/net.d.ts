// plinth:net: HTTP fetch (SPEC.md §8.5, §11). A request goes only to a
// host that a declared, granted `net:<host>` capability allows (or, for a
// private/loopback address, `net.local`). A denied or failed call never
// throws: `done` is called with `response.ok === false` and `response
// .error` set (for example `"denied:undeclared"`, or `"network: ..."`).
// Text bodies only (UTF-8), with a size limit (8 MiB).

/** The result `fetch` passes to `done`. */
export interface Response {
  /** `true` for a 2xx status and no transport error. */
  ok: boolean;
  /** The HTTP status code, or `0` if the request never reached the host
   * (denied, or a network error). */
  status: number;
  /** The response body as text, or `""` on error. */
  text: string;
  /** `null` when `ok`, otherwise a reason such as `"denied:undeclared"`,
   * `"denied:refused"` or `"network: ..."`. */
  error: string | null;
}

/** `fetch`'s `options`: `null` for a plain `GET`, or an object literal
 * (not an arbitrary expression yet) with any of these fields. */
export type FetchOptions = {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  headers?: Map<string, string>;
  body?: string;
};

/** Requests `url` and calls `done` with the result once it completes
 * (SPEC.md §8.4). Never throws. */
export declare function fetch(url: string, options: FetchOptions | null, done: (response: Response) => void): void;
