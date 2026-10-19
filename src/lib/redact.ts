/**
 * Masks credentials in captured headers before they leave the app as a
 * clipboard dump, a cURL command or an exported file. A dump pasted into a
 * bug report or a chat otherwise carries a live session.
 *
 * Only values are masked; header names stay, and so do the parts of a value
 * that help reading the dump: the auth scheme (`Bearer`), cookie names and
 * `Set-Cookie` attributes.
 */

import type { HeaderDto } from "@/ipc/types";

export const SECRET_PLACEHOLDER = "{{TOKEN}}";

// Not a bare `auth`: WWW-Authenticate / Proxy-Authenticate carry the 401
// challenge, which is what a dump of a failed call is for. Names like
// ClientID, ProfileId or Request-Id stay readable: they identify, not grant.
const SECRET_NAME =
  /token|secret|passw|api[-_]?key|authoriz|(^|[-_])auth([-_]|$)|session|csrf|xsrf/i;

export function isSecretHeader(name: string): boolean {
  const n = name.toLowerCase();
  return n === "cookie" || n === "set-cookie" || SECRET_NAME.test(n);
}

function maskCookiePair(pair: string): string {
  const eq = pair.indexOf("=");
  if (eq < 0) return pair;
  return `${pair.slice(0, eq + 1)}${SECRET_PLACEHOLDER}`;
}

export function redactHeaderValue(name: string, value: string): string {
  if (value === "" || !isSecretHeader(name)) return value;
  const n = name.toLowerCase();
  if (n === "cookie") {
    return value
      .split(";")
      .map((part) => {
        const lead = part.match(/^\s*/)![0];
        return lead + maskCookiePair(part.slice(lead.length));
      })
      .join(";");
  }
  if (n === "set-cookie") {
    // Only the first segment is the cookie; Path, Expires, HttpOnly… are not secret.
    const semi = value.indexOf(";");
    return semi < 0
      ? maskCookiePair(value)
      : maskCookiePair(value.slice(0, semi)) + value.slice(semi);
  }
  if (n.includes("authorization")) {
    const scheme = value.match(/^([A-Za-z][\w-]*)\s+\S/);
    if (scheme) return `${scheme[1]} ${SECRET_PLACEHOLDER}`;
  }
  return SECRET_PLACEHOLDER;
}

/** `masked` counts headers whose value actually changed, so the caller can
 *  tell the user something was hidden without claiming it when nothing was. */
export function redactHeaders(headers: readonly HeaderDto[]): {
  headers: HeaderDto[];
  masked: number;
} {
  let masked = 0;
  const out = headers.map((h) => {
    const value = redactHeaderValue(h.name, h.value);
    if (value !== h.value) masked += 1;
    return { name: h.name, value };
  });
  return { headers: out, masked };
}
