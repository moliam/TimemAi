import { createLucideIcon } from "lucide-react";

/*!
 * Lucide database-search, https://github.com/lucide-icons/lucide/blob/main/icons/database-search.svg
 * Backport for lucide-react 0.468 (which does not export DatabaseSearch).
 * ISC License — Copyright (c) 2026 Lucide Icons and Contributors
 * Permission to use, copy, modify, and/or distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
export const DatabaseSearch = createLucideIcon("DatabaseSearch", [
  ["path", { d: "M21 11.693V5", key: "top" }],
  ["path", { d: "m22 22-1.875-1.875", key: "handle" }],
  ["path", { d: "M3 12a9 3 0 0 0 8.697 2.998", key: "middle" }],
  ["path", { d: "M3 5v14a9 3 0 0 0 9.28 2.999", key: "bottom" }],
  ["circle", { cx: "18", cy: "18", r: "3", key: "search" }],
  ["ellipse", { cx: "12", cy: "5", rx: "9", ry: "3", key: "database" }],
]);
